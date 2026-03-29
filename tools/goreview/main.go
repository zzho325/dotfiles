// goreview prints a diff-annotated call graph for Go packages.
//
// Usage:
//
//	goreview [--diff base] [--depth N] packages...
//
// Examples:
//
//	goreview ./pkg/swift/zfp/...                    # full call graph
//	goreview --diff origin/main ./pkg/swift/zfp/... # annotate new/modified
//	goreview --depth 2 ./pkg/transfers/...          # limit depth
package main

import (
	"flag"
	"fmt"
	"os"
	"os/exec"
	"strconv"
	"strings"
)

func main() {
	base := flag.String("diff", "", "base ref for diff (e.g., origin/main)")
	depth := flag.Int("depth", 0, "max call depth from roots (0 = unlimited)")
	changesOnly := flag.Bool("changes-only", false, "only show subtrees with new/modified functions (requires --diff)")
	flag.Parse()

	patterns := flag.Args()
	if len(patterns) == 0 {
		fmt.Fprintln(os.Stderr, "usage: goreview [--diff base] [--depth N] packages...")
		os.Exit(1)
	}

	g, err := loadAndAnalyze(patterns)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	var di *diffInfo
	if *base != "" {
		di, err = getDiff(*base)
		if err != nil {
			fmt.Fprintf(os.Stderr, "diff error: %v\n", err)
			os.Exit(1)
		}
	}

	render(os.Stdout, g, di, *depth, *changesOnly)
}

// diffInfo tracks which files/lines are new or modified relative to a base ref.
type diffInfo struct {
	repoRoot string
	newFiles map[string]bool        // relative paths of added files
	hunks    map[string][]lineRange // relative path → changed line ranges
}

type lineRange struct {
	start, end int
	pureAdd    bool // old-side count is 0 — entire hunk is new code
}

type diffStatus int

const (
	unchanged diffStatus = iota
	added
	modified
)

func (s diffStatus) marker() string {
	switch s {
	case added:
		return "+"
	case modified:
		return "~"
	default:
		return " "
	}
}

func (d *diffInfo) classify(fn *funcNode) diffStatus {
	if d == nil {
		return unchanged
	}
	rel := fn.file
	if strings.HasPrefix(rel, d.repoRoot+"/") {
		rel = rel[len(d.repoRoot)+1:]
	}
	if d.newFiles[rel] {
		return added
	}
	// Check if the function is fully covered by pure-addition hunks.
	// If so, it's a new function in a modified file.
	overlaps := false
	coveredLines := 0
	allPureAdd := true
	fnLen := fn.endLine - fn.line + 1
	for _, h := range d.hunks[rel] {
		if fn.line <= h.end && fn.endLine >= h.start {
			overlaps = true
			if !h.pureAdd {
				allPureAdd = false
			}
			// Count how many of the function's lines this hunk covers.
			lo := h.start
			if fn.line > lo {
				lo = fn.line
			}
			hi := h.end
			if fn.endLine < hi {
				hi = fn.endLine
			}
			coveredLines += hi - lo + 1
		}
	}
	if !overlaps {
		return unchanged
	}
	if allPureAdd && coveredLines >= fnLen {
		return added
	}
	return modified
}

func getDiff(base string) (*diffInfo, error) {
	rootBytes, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		return nil, fmt.Errorf("git rev-parse: %w", err)
	}

	di := &diffInfo{
		repoRoot: strings.TrimSpace(string(rootBytes)),
		newFiles: map[string]bool{},
		hunks:    map[string][]lineRange{},
	}

	// Classify files as added vs modified.
	out, err := exec.Command("git", "diff", "--name-status", base, "--", "*.go").Output()
	if err != nil {
		return nil, fmt.Errorf("git diff --name-status: %w", err)
	}
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		parts := strings.Fields(line)
		if len(parts) >= 2 && parts[0] == "A" {
			di.newFiles[parts[len(parts)-1]] = true
		}
	}

	// Get changed line ranges for modified (non-new) files.
	out, _ = exec.Command("git", "diff", "--unified=0", base, "--", "*.go").Output()
	var curFile string
	for _, line := range strings.Split(string(out), "\n") {
		switch {
		case strings.HasPrefix(line, "+++ b/"):
			curFile = line[6:]
		case strings.HasPrefix(line, "@@") && curFile != "" && !di.newFiles[curFile]:
			if h := parseHunk(line); h.start > 0 {
				di.hunks[curFile] = append(di.hunks[curFile], h)
			}
		}
	}

	return di, nil
}

// parseHunk extracts the new-side line range from a unified diff hunk header.
// Format: @@ -old[,count] +new[,count] @@
func parseHunk(line string) lineRange {
	// Parse old side to detect pure additions (old count = 0).
	minus := strings.Index(line, "-")
	oldCount := 1
	if minus >= 0 {
		oldRest := line[minus+1:]
		if sp := strings.IndexByte(oldRest, ' '); sp >= 0 {
			oldRest = oldRest[:sp]
		}
		oldParts := strings.SplitN(oldRest, ",", 2)
		if len(oldParts) > 1 {
			oldCount, _ = strconv.Atoi(oldParts[1])
		}
	}

	plus := strings.Index(line, "+")
	if plus < 0 {
		return lineRange{}
	}
	rest := line[plus+1:]
	if sp := strings.IndexByte(rest, ' '); sp >= 0 {
		rest = rest[:sp]
	}
	parts := strings.SplitN(rest, ",", 2)
	start, err := strconv.Atoi(parts[0])
	if err != nil {
		return lineRange{}
	}
	count := 1
	if len(parts) > 1 {
		count, _ = strconv.Atoi(parts[1])
	}
	if count == 0 {
		return lineRange{} // pure deletion
	}
	return lineRange{
		start:   start,
		end:     start + count - 1,
		pureAdd: oldCount == 0,
	}
}
