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

	// Build adjacency list: caller → callees (deduplicated, ordered by callee line).
	adj := map[*funcInfo][]*funcInfo{}
	seen := map[[2]*funcInfo]bool{}
	for _, e := range edges {
		key := [2]*funcInfo{e.caller, e.callee}
		if seen[key] {
			continue
		}
		seen[key] = true
		adj[e.caller] = append(adj[e.caller], e.callee)
	}
	for _, children := range adj {
		sort.Slice(children, func(i, j int) bool {
			return children[i].line < children[j].line
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
	adj map[*funcInfo][]*funcInfo,
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

	for i, child := range children {
		last := i == len(children)-1
		printNode(buf, child, childPrefix, last, false, adj, violations, visited)
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
