package main

import (
	"fmt"
	"io"
	"sort"
	"strings"
)

func render(w io.Writer, g *callGraph, di *diffInfo, maxDepth int) {
	// Header with stats.
	fmt.Fprintf(w, "%s", g.pkgPath)
	if di != nil {
		var nNew, nMod int
		for _, fn := range g.funcs {
			switch di.classify(fn) {
			case added:
				nNew++
			case modified:
				nMod++
			}
		}
		var parts []string
		if nNew == len(g.funcs) {
			parts = append(parts, fmt.Sprintf("new package, %d functions", len(g.funcs)))
		} else {
			if nNew > 0 {
				parts = append(parts, fmt.Sprintf("%d new", nNew))
			}
			if nMod > 0 {
				parts = append(parts, fmt.Sprintf("%d modified", nMod))
			}
			parts = append(parts, fmt.Sprintf("%d total", len(g.funcs)))
		}
		fmt.Fprintf(w, " — %s", strings.Join(parts, ", "))
	}
	fmt.Fprintf(w, "\n\n")

	// Which functions are called by something?
	called := map[*funcNode]bool{}
	for _, children := range g.adj {
		for _, c := range children {
			called[c.fn] = true
		}
	}

	// Group exported functions: methods by receiver, rest standalone.
	// A type gets its own group if it has at least one uncalled exported method
	// (i.e., an API entry point). Types whose exported methods are *only* ever
	// called from other functions (e.g., Config.Validate) appear inline.
	typeHasRoot := map[string]bool{}
	for _, fn := range g.funcs {
		if fn.exported && fn.recv != "" && !called[fn] {
			typeHasRoot[fn.recv] = true
		}
	}
	typeRoots := map[string][]*funcNode{} // recv type → exported methods
	var standalone []*funcNode
	for _, fn := range g.funcs {
		if !fn.exported {
			continue
		}
		if fn.recv != "" && typeHasRoot[fn.recv] {
			typeRoots[fn.recv] = append(typeRoots[fn.recv], fn)
		} else if fn.recv == "" {
			standalone = append(standalone, fn)
		}
	}

	sort.Slice(standalone, func(i, j int) bool {
		return standalone[i].line < standalone[j].line
	})

	visited := map[*funcNode]bool{}

	// Standalone exported functions.
	for _, fn := range standalone {
		printFuncNode(w, fn, "  ", true, true, "", g.adj, di, visited, 0, maxDepth)
	}
	if len(standalone) > 0 && len(typeRoots) > 0 {
		fmt.Fprintln(w)
	}

	// Type groups.
	typeNames := sortedKeys(typeRoots)
	for ti, tname := range typeNames {
		roots := typeRoots[tname]
		sort.Slice(roots, func(i, j int) bool {
			return roots[i].line < roots[j].line
		})
		fmt.Fprintf(w, "  %s\n", tname)
		for i, fn := range roots {
			isLast := i == len(roots)-1
			printFuncNode(w, fn, "  ", isLast, false, tname, g.adj, di, visited, 0, maxDepth)
		}
		if ti < len(typeNames)-1 {
			fmt.Fprintln(w)
		}
	}
	fmt.Fprintln(w)
}

func printFuncNode(
	w io.Writer,
	fn *funcNode,
	prefix string,
	isLast bool,
	isRoot bool,
	groupRecv string, // non-empty when inside a type group → strip receiver prefix
	adj map[*funcNode][]callChild,
	di *diffInfo,
	visited map[*funcNode]bool,
	depth, maxDepth int,
) {
	var marker string
	if di != nil {
		marker = di.classify(fn).marker()
	} else {
		marker = " "
	}

	var connector string
	switch {
	case isRoot:
		connector = ""
	case isLast:
		connector = "└── "
	default:
		connector = "├── "
	}

	// Display name: strip receiver prefix when inside a type group.
	name := fn.name
	if groupRecv != "" && fn.recv == groupRecv {
		name = fn.shortName
	}

	// Revisit: show name only with ↩.
	if visited[fn] {
		fmt.Fprintf(w, "%s%s%s%s ↩\n", marker, prefix, connector, name)
		return
	}
	visited[fn] = true

	label := name + fn.sig
	if !fn.exported {
		label += "  [unexported]"
	}
	fmt.Fprintf(w, "%s%s%s%s\n", marker, prefix, connector, label)

	if maxDepth > 0 && depth >= maxDepth {
		return
	}

	children := adj[fn]
	if len(children) == 0 {
		return
	}

	var childPrefix string
	switch {
	case isRoot:
		childPrefix = prefix
	case isLast:
		childPrefix = prefix + "    "
	default:
		childPrefix = prefix + "│   "
	}

	for i, c := range children {
		last := i == len(children)-1
		printFuncNode(w, c.fn, childPrefix, last, false, groupRecv, adj, di, visited, depth+1, maxDepth)
	}
}

func sortedKeys(m map[string][]*funcNode) []string {
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	return keys
}
