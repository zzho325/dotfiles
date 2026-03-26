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
	callerCallees := map[*funcInfo][]callSite{}
	seen := map[[2]*funcInfo]bool{}
	for _, e := range edges {
		if inCycle[e.caller] && inCycle[e.callee] {
			continue
		}
		// Skip nested calls — e.g. makeBatch(t, testBatchHeader()) where
		// testBatchHeader is an argument, not an independent sibling call.
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

	// For each caller, sort callees by call-site line (call order).
	// Then check: if A is called before B, A should be defined before B.
	var violations []siblingViolation
	for caller, sites := range callerCallees {
		sort.Slice(sites, func(i, j int) bool {
			return sites[i].callLine < sites[j].callLine
		})
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
