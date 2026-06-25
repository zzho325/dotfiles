package analyzer

import (
	"fmt"
	"go/token"
	"path/filepath"
	"sort"
	"strings"
	"sync"
)

// printedFiles tracks files already printed across package analyses
// to avoid duplicates (test packages re-analyze source files).
var (
	printedMu    sync.Mutex
	printedFiles = map[string]bool{}
)

// child pairs a callee with the line where it's called from its parent.
type child struct {
	fi       *funcInfo
	callLine int
}

// printCallTree prints an ASCII call tree for a single file's functions.
func printCallTree(
	fset *token.FileSet,
	fileName string,
	funcs []*funcInfo,
	edges []edge,
	violations map[*funcInfo]*funcInfo, // callee → earliest caller that it violates
) {
	// Skip generated/temp files (test binary wrappers have hash names).
	if !strings.HasSuffix(fileName, ".go") {
		return
	}

	// Skip files already printed by a prior package analysis.
	printedMu.Lock()
	if printedFiles[fileName] {
		printedMu.Unlock()
		return
	}
	printedFiles[fileName] = true
	printedMu.Unlock()

	// Build adjacency list: caller → callees, ordered by call-site line.
	// Not deduplicated — repeat calls show as ↩ in the tree.
	adj := map[*funcInfo][]child{}
	for _, e := range edges {
		adj[e.caller] = append(adj[e.caller], child{fi: e.callee, callLine: e.callLine})
	}
	for _, children := range adj {
		sort.Slice(children, func(i, j int) bool {
			return children[i].callLine < children[j].callLine
		})
	}

	// Find entry points: functions not called by anything in this file.
	called := map[*funcInfo]bool{}
	for _, e := range edges {
		called[e.callee] = true
	}
	var roots []*funcInfo
	for _, fi := range funcs {
		if !called[fi] {
			roots = append(roots, fi)
		}
	}
	sort.Slice(roots, func(i, j int) bool {
		return roots[i].line < roots[j].line
	})

	// Buffer output, then print atomically to avoid interleaving.
	var buf strings.Builder
	shortName := filepath.Base(fileName)
	fmt.Fprintf(&buf, "%s:\n", shortName)

	visited := map[*funcInfo]bool{}
	for _, root := range roots {
		printNode(&buf, root, "", true, true, adj, violations, visited)
	}
	buf.WriteString("\n")

	fmt.Print(buf.String())
}

// printNode recursively prints a node in the call tree with box-drawing connectors.
func printNode(
	buf *strings.Builder,
	fi *funcInfo,
	prefix string,
	isLast bool,
	isRoot bool,
	adj map[*funcInfo][]child,
	violations map[*funcInfo]*funcInfo,
	visited map[*funcInfo]bool,
) {
	var connector string
	if isRoot {
		connector = "  "
	} else if isLast {
		connector = "  └── "
	} else {
		connector = "  ├── "
	}

	label := fmt.Sprintf("%s (line %d)", fi.name, fi.line)
	if caller, ok := violations[fi]; ok {
		label += fmt.Sprintf("  ✗ before caller %s at line %d", caller.name, caller.line)
	}

	if visited[fi] && len(adj[fi]) > 0 {
		fmt.Fprintf(buf, "%s%s%s  ↩\n", prefix, connector, fi.name)
		return
	}
	fmt.Fprintf(buf, "%s%s%s\n", prefix, connector, label)
	visited[fi] = true

	children := adj[fi]
	if len(children) == 0 {
		return
	}

	var childPrefix string
	if isRoot {
		childPrefix = prefix + "  "
	} else if isLast {
		childPrefix = prefix + "      "
	} else {
		childPrefix = prefix + "  │   "
	}

	for i, c := range children {
		last := i == len(children)-1
		printNode(buf, c.fi, childPrefix, last, false, adj, violations, visited)
	}
}

// buildViolationMap returns a map of callee → earliest caller where callee.line < caller.line.
func buildViolationMap(
	edges []edge,
	inCycle map[*funcInfo]bool,
) map[*funcInfo]*funcInfo {
	earliestCaller := map[*funcInfo]*funcInfo{}
	for _, e := range edges {
		if inCycle[e.caller] && inCycle[e.callee] {
			continue
		}
		prev, exists := earliestCaller[e.callee]
		if !exists || e.caller.line < prev.line {
			earliestCaller[e.callee] = e.caller
		}
	}

	violations := map[*funcInfo]*funcInfo{}
	for callee, caller := range earliestCaller {
		if callee.line < caller.line {
			violations[callee] = caller
		}
	}
	return violations
}

// siblingViolation records a callee that is defined out of order
// relative to a sibling callee from the same caller.
type siblingViolation struct {
	callee      *funcInfo // the out-of-order callee
	calleeCall  int       // call-site line of callee in caller
	sibling     *funcInfo // the sibling it should appear after
	siblingCall int       // call-site line of sibling in caller
	caller      *funcInfo // their shared caller
}

// buildSiblingViolations checks that callees of the same caller are defined
// in the same order they are called. If caller C calls A at line 10 then B
// at line 20, A's definition should appear before B's definition.
func buildSiblingViolations(
	edges []edge,
	inCycle map[*funcInfo]bool,
) []siblingViolation {
	// Group edges by caller, keeping only the first call-site per callee.
	type callSite struct {
		callee   *funcInfo
		callLine int
	}

	// allCallerCallees includes nested calls — used to detect
	// contradictory orderings across callers even when one caller's
	// call is nested (e.g. insertRecords(t, createBatch(t))).
	allCallerCallees := map[*funcInfo][]callSite{}
	allSeen := map[[2]*funcInfo]bool{}

	// callerCallees excludes nested calls — used for violation reporting
	// (nested calls aren't independent sequential siblings).
	callerCallees := map[*funcInfo][]callSite{}
	seen := map[[2]*funcInfo]bool{}

	for _, e := range edges {
		if inCycle[e.caller] && inCycle[e.callee] {
			continue
		}
		akey := [2]*funcInfo{e.caller, e.callee}
		if !allSeen[akey] {
			allSeen[akey] = true
			allCallerCallees[e.caller] = append(
				allCallerCallees[e.caller],
				callSite{callee: e.callee, callLine: e.callLine},
			)
		}
		if e.nested {
			continue
		}
		key := [2]*funcInfo{e.caller, e.callee}
		if seen[key] {
			continue
		}
		seen[key] = true
		callerCallees[e.caller] = append(
			callerCallees[e.caller],
			callSite{callee: e.callee, callLine: e.callLine},
		)
	}

	// Sort both maps by call-site line.
	for _, sites := range allCallerCallees {
		sort.Slice(sites, func(i, j int) bool {
			return sites[i].callLine < sites[j].callLine
		})
	}
	for _, sites := range callerCallees {
		sort.Slice(sites, func(i, j int) bool {
			return sites[i].callLine < sites[j].callLine
		})
	}

	// Collect implied orderings and first-call-site per callee from
	// ALL edges (including nested). A nested call like createBatch
	// inside insertRecords(t, createBatch(t)) still establishes that
	// createBatch appears before later siblings in the same caller.
	type funcPair struct{ first, second *funcInfo }
	impliedOrder := map[funcPair]bool{}
	firstCallLine := map[*funcInfo]int{}
	for _, sites := range allCallerCallees {
		for i := 0; i < len(sites); i++ {
			if prev, ok := firstCallLine[sites[i].callee]; !ok || sites[i].callLine < prev {
				firstCallLine[sites[i].callee] = sites[i].callLine
			}
			for j := i + 1; j < len(sites); j++ {
				a, b := sites[i].callee, sites[j].callee
				if a == b || isMethod(a) != isMethod(b) {
					continue
				}
				impliedOrder[funcPair{a, b}] = true
			}
		}
	}

	// Check: if A is called before B (same caller), A should be defined before B.
	var violations []siblingViolation
	for caller, sites := range callerCallees {
		// For each callee, check against its immediate predecessor in
		// call order. Only report once per callee (not against every sibling).
		reported := map[*funcInfo]bool{}
		for i := 1; i < len(sites); i++ {
			prev := sites[i-1] // called earlier
			cur := sites[i]    // called later
			if prev.callee == cur.callee || reported[prev.callee] {
				continue
			}
			// Skip when siblings cross the method/function boundary —
			// methods and standalone functions belong to different ordering tiers.
			if isMethod(prev.callee) != isMethod(cur.callee) {
				continue
			}
			// When another caller imposes the opposite ordering, the
			// constraints conflict. The callee called earliest across
			// the file wins — skip if this caller's ordering disagrees
			// with the first-call-site ordering.
			if impliedOrder[funcPair{cur.callee, prev.callee}] &&
				firstCallLine[cur.callee] <= firstCallLine[prev.callee] {
				continue
			}
			if prev.callee.line > cur.callee.line {
				reported[prev.callee] = true
				violations = append(violations, siblingViolation{
					callee:      prev.callee,
					calleeCall:  prev.callLine,
					sibling:     cur.callee,
					siblingCall: cur.callLine,
					caller:      caller,
				})
			}
		}
	}
	return violations
}

// callerGroupViolation records a function that is defined out of order
// relative to another function whose primary caller appears earlier.
type callerGroupViolation struct {
	fn          *funcInfo // the out-of-order function
	fnCaller    *funcInfo // its primary caller
	other       *funcInfo // the function it should appear after
	otherCaller *funcInfo // the other's primary caller
}

// buildCallerGroupViolations checks that callees of different callers are
// ordered consistently with their callers. If FirstEntry (line 10) calls
// helperA, and SecondEntry (line 20) calls helperB, then helperA should
// appear before helperB.
func buildCallerGroupViolations(
	edges []edge,
	inCycle map[*funcInfo]bool,
	existingViolations map[*funcInfo]*funcInfo,
) []callerGroupViolation {
	// Find the sole caller for each callee. If a function is called by
	// multiple different callers, its group ordering is ambiguous — skip it.
	callerCount := map[*funcInfo]map[*funcInfo]bool{}
	for _, e := range edges {
		if inCycle[e.caller] && inCycle[e.callee] {
			continue
		}
		if callerCount[e.callee] == nil {
			callerCount[e.callee] = map[*funcInfo]bool{}
		}
		callerCount[e.callee][e.caller] = true
	}
	primaryCaller := map[*funcInfo]*funcInfo{}
	for callee, callers := range callerCount {
		if len(callers) != 1 {
			continue // shared helper — ambiguous ordering
		}
		for caller := range callers {
			primaryCaller[callee] = caller
		}
	}

	// Build adjacency list with children sorted by call-site line (DFS order).
	type callChild struct {
		fn       *funcInfo
		callLine int
	}
	adj := map[*funcInfo][]callChild{}
	called := map[*funcInfo]bool{}
	for _, e := range edges {
		if inCycle[e.caller] && inCycle[e.callee] {
			continue
		}
		called[e.callee] = true
		adj[e.caller] = append(adj[e.caller], callChild{e.callee, e.callLine})
	}
	for _, children := range adj {
		sort.Slice(children, func(i, j int) bool {
			return children[i].callLine < children[j].callLine
		})
	}

	// Find entry points (not called by anything), sorted by line.
	var roots []*funcInfo
	seen := map[*funcInfo]bool{}
	for fn := range adj {
		if !called[fn] && !seen[fn] {
			roots = append(roots, fn)
			seen[fn] = true
		}
	}
	for fn := range primaryCaller {
		if !called[fn] && !seen[fn] {
			roots = append(roots, fn)
			seen[fn] = true
		}
	}
	sort.Slice(roots, func(i, j int) bool {
		return roots[i].line < roots[j].line
	})

	// Compute DFS pre-order position and exit time for each function.
	// Entry/exit times allow O(1) ancestry checks: A is an ancestor of B
	// iff dfsEntry[A] < dfsEntry[B] && dfsExit[A] > dfsExit[B].
	dfsPos := map[*funcInfo]int{}
	dfsExit := map[*funcInfo]int{}
	pos := 0
	var dfs func(*funcInfo)
	dfs = func(fn *funcInfo) {
		if _, ok := dfsPos[fn]; ok {
			return
		}
		dfsPos[fn] = pos
		pos++
		for _, c := range adj[fn] {
			dfs(c.fn)
		}
		dfsExit[fn] = pos
		pos++
	}
	for _, root := range roots {
		dfs(root)
	}

	isAncestor := func(a, b *funcInfo) bool {
		ae, aok := dfsPos[a]
		be, bok := dfsPos[b]
		if !aok || !bok {
			return false
		}
		return ae < be && dfsExit[a] > dfsExit[b]
	}

	// Compute depth for each callee by walking up the primaryCaller chain.
	// Only compare callees at the same depth — comparing across depths
	// creates false positives (e.g. a handler's deep helper vs a sibling
	// handler's direct helper in service files).
	depth := map[*funcInfo]int{}
	var computeDepth func(*funcInfo) int
	computeDepth = func(fn *funcInfo) int {
		if d, ok := depth[fn]; ok {
			return d
		}
		caller, hasCaller := primaryCaller[fn]
		if !hasCaller {
			depth[fn] = 0
			return 0
		}
		d := computeDepth(caller) + 1
		depth[fn] = d
		return d
	}
	for callee := range primaryCaller {
		computeDepth(callee)
	}

	// Collect single-caller callees sorted by definition line.
	var callees []*funcInfo
	for callee := range primaryCaller {
		callees = append(callees, callee)
	}
	sort.Slice(callees, func(i, j int) bool {
		return callees[i].line < callees[j].line
	})

	// For each pair of single-caller callees with different callers
	// at the same depth, check if their file order matches DFS pre-order.
	var violations []callerGroupViolation
	for i := 0; i < len(callees); i++ {
		for j := i + 1; j < len(callees); j++ {
			x := callees[i] // appears first in file
			y := callees[j] // appears second in file
			if primaryCaller[x] == primaryCaller[y] {
				continue // same caller — handled by sibling check
			}
			if isMethod(x) != isMethod(y) {
				continue
			}
			// Only compare callees at the same depth.
			if depth[x] != depth[y] {
				continue
			}
			// Skip when one caller is an ancestor of the other —
			// they're at different depths and comparing their
			// callees' order is not meaningful.
			cx, cy := primaryCaller[x], primaryCaller[y]
			if isAncestor(cx, cy) || isAncestor(cy, cx) {
				continue
			}
			// Skip if already flagged by caller-before-callee check.
			if _, ok := existingViolations[x]; ok {
				continue
			}
			if _, ok := existingViolations[y]; ok {
				continue
			}
			// x appears before y in file. If DFS visits y before x,
			// then x is in the wrong position.
			xPos, xOk := dfsPos[x]
			yPos, yOk := dfsPos[y]
			if !xOk || !yOk {
				continue
			}
			if xPos > yPos {
				violations = append(violations, callerGroupViolation{
					fn:          x,
					fnCaller:    primaryCaller[x],
					other:       y,
					otherCaller: primaryCaller[y],
				})
				break // only report first violation per function
			}
		}
	}
	return violations
}
