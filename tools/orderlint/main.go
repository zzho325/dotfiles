package main

import (
	"os"
	"path/filepath"
	"strings"

	"orderlint/analyzer"

	"golang.org/x/tools/go/analysis/singlechecker"
)

func main() {
	// Replace .go file arguments with their parent directories so that
	// files from multiple packages can be analyzed in one invocation.
	// singlechecker treats .go args as a single ad-hoc package, which
	// fails when files span multiple directories.
	seen := map[string]bool{}
	var newArgs []string
	for _, arg := range os.Args[1:] {
		if !strings.HasPrefix(arg, "-") && strings.HasSuffix(arg, ".go") {
			dir := filepath.Dir(arg)
			if !seen[dir] {
				seen[dir] = true
				newArgs = append(newArgs, dir)
			}
			continue
		}
		newArgs = append(newArgs, arg)
	}
	os.Args = append([]string{os.Args[0]}, newArgs...)

	singlechecker.Main(analyzer.Analyzer)
}
