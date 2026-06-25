package analyzer

import (
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"

	"golang.org/x/tools/go/analysis"
	"golang.org/x/tools/go/analysis/passes/inspect"
)

var graphFlag bool
var baselineFlag string

var Analyzer = &analysis.Analyzer{
	Name:     "orderlint",
	Doc:      "checks that functions appear in call-order: if A calls B, A should be defined before B",
	Requires: []*analysis.Analyzer{inspect.Analyzer},
	Run:      run,
}

func init() {
	Analyzer.Flags.BoolVar(
		&graphFlag, "graph", false,
		"print ASCII call tree per file instead of diagnostics",
	)
	Analyzer.Flags.StringVar(
		&baselineFlag, "baseline", "",
		"path to baseline file of accepted violations (file:Func per line)",
	)
}

var (
	baselineOnce sync.Once
	baselineSet  map[string]bool
)

func repoRoot() string {
	out, err := exec.Command("git", "rev-parse", "--show-toplevel").Output()
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(out))
}

func loadBaseline() {
	baselineSet = map[string]bool{}
	path := baselineFlag
	if path == "" {
		if root := repoRoot(); root != "" {
			path = filepath.Join(root, ".orderlintbaseline")
		} else {
			path = ".orderlintbaseline"
		}
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return
	}
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		baselineSet[line] = true
	}
}

// isBaselined returns true if the function is listed in the baseline file.
func isBaselined(fileName, funcName string) bool {
	baselineOnce.Do(loadBaseline)
	return baselineSet[filepath.Base(fileName)+":"+funcName]
}

// funcInfo tracks a function declaration's identity and position.
type funcInfo struct {
	name string    // display name: "Foo" or "RecvType.Method"
	obj  types.Object
	pos  token.Pos
	line int
	decl *ast.FuncDecl
}

// edge represents a caller→callee relationship within a file.
// callLine is the source line of the call site within the caller's body.
type edge struct {
	caller   *funcInfo
	callee   *funcInfo
	callLine int  // line number of the call expression in the caller
	nested   bool // true if this call is inside another call's argument list
}

func run(pass *analysis.Pass) (interface{}, error) {
	fset := pass.Fset

	// Group function declarations by file.
	fileDecls := map[string][]*funcInfo{}
	objToFunc := map[types.Object]*funcInfo{}

	for _, file := range pass.Files {
		fileName := fset.Position(file.Pos()).Filename
		for _, decl := range file.Decls {
			fd, ok := decl.(*ast.FuncDecl)
			if !ok || fd.Name == nil {
				continue
			}
			obj := pass.TypesInfo.Defs[fd.Name]
			if obj == nil {
				continue
			}
			fi := &funcInfo{
				name: funcDisplayName(fd),
				obj:  obj,
				pos:  fd.Pos(),
				line: fset.Position(fd.Pos()).Line,
				decl: fd,
			}
			fileDecls[fileName] = append(fileDecls[fileName], fi)
			objToFunc[obj] = fi
		}
	}

	// Analyze each file independently.
	for fileName, funcs := range fileDecls {
		isTest := strings.HasSuffix(fileName, "_test.go")
		analyzeFile(pass, fileName, funcs, objToFunc, fileDecls, isTest)
	}

	return nil, nil
}

// analyzeFile checks function ordering within a single file.
func analyzeFile(
	pass *analysis.Pass,
	fileName string,
	funcs []*funcInfo,
	objToFunc map[types.Object]*funcInfo,
	fileDecls map[string][]*funcInfo,
	isTest bool,
) {
	fset := pass.Fset

	// Build intra-file call graph.
	edges := buildCallGraph(pass, funcs, objToFunc)

	// Detect cycles — collect functions involved in cycles.
	inCycle := detectCycles(pass, funcs, edges)

	// Build violation map (shared by both diagnostic and graph modes).
	violations := buildViolationMap(edges, inCycle)

	if graphFlag {
		printCallTree(fset, fileName, funcs, edges, violations)
		return
	}

	// Report ordering violations.
	for callee, caller := range violations {
		if isBaselined(fileName, callee.name) {
			continue
		}
		pass.Report(analysis.Diagnostic{
			Pos: callee.pos,
			Message: fmt.Sprintf(
				"%s (line %d) should appear after %s (line %d) — %s calls %s",
				callee.name, fset.Position(callee.pos).Line,
				caller.name, fset.Position(caller.pos).Line,
				caller.name, callee.name,
			),
		})
	}

	// Report sibling ordering violations.
	// Only report if the callee isn't already flagged by caller-before-callee check.
	siblingViolations := buildSiblingViolations(edges, inCycle)
	for _, sv := range siblingViolations {
		if _, alreadyFlagged := violations[sv.callee]; alreadyFlagged {
			continue
		}
		if isBaselined(fileName, sv.callee.name) {
			continue
		}
		pass.Report(analysis.Diagnostic{
			Pos: sv.callee.pos,
			Message: fmt.Sprintf(
				"%s (line %d) should appear before %s (line %d) — %s calls %s (line %d) before %s (line %d)",
				sv.callee.name, fset.Position(sv.callee.pos).Line,
				sv.sibling.name, fset.Position(sv.sibling.pos).Line,
				sv.caller.name,
				sv.callee.name, sv.calleeCall,
				sv.sibling.name, sv.siblingCall,
			),
		})
	}

	// Report caller-group ordering violations (non-test files only).
	// If A's caller appears before B's caller, A should appear before B.
	// Skipped for test files where helpers are commonly shared across tests.
	if !isTest {
		callerGroupViolations := buildCallerGroupViolations(edges, inCycle, violations)
		for _, cgv := range callerGroupViolations {
			if _, alreadyFlagged := violations[cgv.fn]; alreadyFlagged {
				continue
			}
			if isBaselined(fileName, cgv.fn.name) {
				continue
			}
			pass.Report(analysis.Diagnostic{
				Pos: cgv.fn.pos,
				Message: fmt.Sprintf(
					"%s (line %d) should appear after %s (line %d) — %s is reached before %s in the call tree",
					cgv.fn.name, fset.Position(cgv.fn.pos).Line,
					cgv.other.name, fset.Position(cgv.other.pos).Line,
					cgv.other.name, cgv.fn.name,
				),
			})
		}
	}

	// Test file: check that helpers appear after test functions,
	// and that test functions are ordered to match source functions.
	if isTest {
		checkTestHelperOrdering(pass, funcs)
		checkTestFunctionOrdering(pass, fileName, funcs, fileDecls, objToFunc)
	}
}

// buildCallGraph finds all intra-file caller→callee edges.
func buildCallGraph(
	pass *analysis.Pass,
	funcs []*funcInfo,
	objToFunc map[types.Object]*funcInfo,
) []edge {
	// Set of objects declared in this file for fast lookup.
	fileObjs := map[types.Object]bool{}
	for _, fi := range funcs {
		fileObjs[fi.obj] = true
	}

	var edges []edge
	for _, caller := range funcs {
		if caller.decl.Body == nil {
			continue
		}
		callDepth := 0
		var walk func(ast.Node) bool
		walk = func(n ast.Node) bool {
			call, ok := n.(*ast.CallExpr)
			if !ok {
				return true
			}
			callee := resolveCallee(pass, call, fileObjs, objToFunc)
			if callee != nil && callee != caller {
				callLine := pass.Fset.Position(call.Pos()).Line
				edges = append(edges, edge{
					caller:   caller,
					callee:   callee,
					callLine: callLine,
					nested:   callDepth > 0,
				})
			}
			// Walk arguments at incremented depth.
			callDepth++
			for _, arg := range call.Args {
				ast.Inspect(arg, walk)
			}
			callDepth--
			return false // already walked children
		}
		ast.Inspect(caller.decl.Body, walk)
	}
	return edges
}

// resolveCallee resolves a call expression to a funcInfo if the callee
// is declared in the same file.
func resolveCallee(
	pass *analysis.Pass,
	call *ast.CallExpr,
	fileObjs map[types.Object]bool,
	objToFunc map[types.Object]*funcInfo,
) *funcInfo {
	var ident *ast.Ident
	switch fn := call.Fun.(type) {
	case *ast.Ident:
		// Direct call: foo()
		ident = fn
	case *ast.SelectorExpr:
		// Method or qualified call: x.Foo()
		ident = fn.Sel
	default:
		return nil
	}

	obj := pass.TypesInfo.Uses[ident]
	if obj == nil {
		return nil
	}
	if !fileObjs[obj] {
		return nil
	}
	return objToFunc[obj]
}

// detectCycles finds functions involved in cycles via DFS 3-color marking.
// Returns the set of functions that are part of at least one cycle.
func detectCycles(
	pass *analysis.Pass,
	funcs []*funcInfo,
	edges []edge,
) map[*funcInfo]bool {
	// Build adjacency list.
	adj := map[*funcInfo][]*funcInfo{}
	for _, e := range edges {
		adj[e.caller] = append(adj[e.caller], e.callee)
	}

	const (
		white = 0 // unvisited
		gray  = 1 // in current DFS path
		black = 2 // fully processed
	)

	color := map[*funcInfo]int{}
	inCycle := map[*funcInfo]bool{}
	// parent tracks the DFS path for cycle reporting.
	parent := map[*funcInfo]*funcInfo{}

	var dfs func(node *funcInfo)
	dfs = func(node *funcInfo) {
		color[node] = gray
		for _, next := range adj[node] {
			switch color[next] {
			case white:
				parent[next] = node
				dfs(next)
			case gray:
				// Back edge — cycle detected. Walk back to mark all nodes in cycle.
				reportCycle(pass, node, next, parent)
				cur := node
				for cur != next {
					inCycle[cur] = true
					cur = parent[cur]
				}
				inCycle[next] = true
			}
		}
		color[node] = black
	}

	for _, fi := range funcs {
		if color[fi] == white {
			dfs(fi)
		}
	}
	return inCycle
}

// reportCycle emits a warning diagnostic for a detected cycle.
func reportCycle(
	pass *analysis.Pass,
	from, to *funcInfo,
	parent map[*funcInfo]*funcInfo,
) {
	// Build cycle path string: to → ... → from → to
	var path []string
	cur := from
	for cur != to {
		path = append([]string{cur.name}, path...)
		cur = parent[cur]
	}
	path = append([]string{to.name}, path...)
	path = append(path, to.name)

	pass.Report(analysis.Diagnostic{
		Pos:     to.pos,
		Message: fmt.Sprintf("cycle detected: %s", strings.Join(path, " → ")),
	})
}

// checkTestHelperOrdering checks that unexported non-test functions in test
// files appear after all exported test/benchmark/example functions.
func checkTestHelperOrdering(pass *analysis.Pass, funcs []*funcInfo) {
	fset := pass.Fset
	lastTestLine := 0
	for _, fi := range funcs {
		if isTestFunc(fi.name) {
			if fi.line > lastTestLine {
				lastTestLine = fi.line
			}
		}
	}
	if lastTestLine == 0 {
		return
	}
	fileName := fset.Position(funcs[0].pos).Filename
	for _, fi := range funcs {
		if !isTestFunc(fi.name) && isUnexported(fi.name) && fi.line < lastTestLine &&
			!isBaselined(fileName, fi.name) {
			pass.Report(analysis.Diagnostic{
				Pos: fi.pos,
				Message: fmt.Sprintf(
					"test helper %s (line %d) should appear after all test functions (last test at line %d)",
					fi.name, fset.Position(fi.pos).Line, lastTestLine,
				),
			})
		}
	}
}

// checkTestFunctionOrdering checks that test functions are ordered to match
// the order of the source functions they test. TestFoo should appear before
// TestBar if Foo is defined before Bar in the source file.
func checkTestFunctionOrdering(
	pass *analysis.Pass,
	testFileName string,
	funcs []*funcInfo,
	fileDecls map[string][]*funcInfo,
	objToFunc map[types.Object]*funcInfo,
) {
	fset := pass.Fset

	// Find the corresponding source file(s) for this test file.
	sourceFiles := findSourceFiles(testFileName, fileDecls)
	if len(sourceFiles) == 0 {
		return
	}

	// Build source function maps: name→location and obj→membership.
	type sourceLoc struct {
		line int
		file string
	}
	sourceOrder := map[string]sourceLoc{}
	sourceObjSet := map[types.Object]bool{}
	for _, sf := range sourceFiles {
		for _, fi := range fileDecls[sf] {
			sourceOrder[fi.name] = sourceLoc{line: fi.line, file: sf}
			sourceObjSet[fi.obj] = true
		}
	}

	// Match each test function to its source function.
	// First try name-based matching, then fall back to call graph analysis.
	type testMatch struct {
		testFunc   *funcInfo
		sourceName string
		sourceLine int
		sourceFile string
	}
	var matches []testMatch
	for _, fi := range funcs {
		if !isTestFunc(fi.name) {
			continue
		}
		// Try name-based matching first.
		matched := false
		candidates := testToSourceCandidates(fi.name)
		for _, srcName := range candidates {
			if loc, ok := sourceOrder[srcName]; ok {
				matches = append(matches, testMatch{
					testFunc:   fi,
					sourceName: srcName,
					sourceLine: loc.line,
					sourceFile: loc.file,
				})
				matched = true
				break
			}
		}
		if matched {
			continue
		}
		// Fall back: find the first source function called by this test.
		target := firstSourceCall(pass, fi, sourceObjSet, objToFunc)
		if target != nil {
			if loc, ok := sourceOrder[target.name]; ok {
				matches = append(matches, testMatch{
					testFunc:   fi,
					sourceName: target.name,
					sourceLine: loc.line,
					sourceFile: loc.file,
				})
			}
		}
	}

	// Check ordering: for each pair of matched test functions,
	// if source A is before source B, test A should be before test B.
	for i := 0; i < len(matches); i++ {
		for j := i + 1; j < len(matches); j++ {
			a, b := matches[i], matches[j]
			// Only compare tests whose source functions are in the same file.
			if a.sourceFile != b.sourceFile {
				continue
			}
			// a appears before b in test file (by iteration order from funcs).
			// If a's source is after b's source, a is out of order.
			if a.sourceLine > b.sourceLine {
				if isBaselined(testFileName, a.testFunc.name) {
					continue
				}
				pass.Report(analysis.Diagnostic{
					Pos: a.testFunc.pos,
					Message: fmt.Sprintf(
						"%s (line %d) should appear after %s (line %d) — source function %s (line %d) is defined before %s (line %d)",
						a.testFunc.name, fset.Position(a.testFunc.pos).Line,
						b.testFunc.name, fset.Position(b.testFunc.pos).Line,
						b.sourceName, b.sourceLine,
						a.sourceName, a.sourceLine,
					),
				})
				break // only report first violation per test function
			}
		}
	}
}

// findSourceFiles returns non-test file paths in the same directory as the
// test file.
func findSourceFiles(
	testFileName string,
	fileDecls map[string][]*funcInfo,
) []string {
	dir := filepath.Dir(testFileName)
	var sources []string
	for fn := range fileDecls {
		if strings.HasSuffix(fn, "_test.go") {
			continue
		}
		if filepath.Dir(fn) == dir {
			sources = append(sources, fn)
		}
	}
	return sources
}

// testToSourceCandidates returns candidate source function names for a test
// function. Returns candidates in priority order: method match first, then
// plain function match.
//
//	TestFoo                → ["Foo"]
//	TestFoo_subtest        → ["Foo.subtest", "Foo"]
//	TestType_Method        → ["Type.Method", "Type"]
//	TestType_Method_sub    → ["Type.Method_sub", "Type.Method", "Type"]
func testToSourceCandidates(testName string) []string {
	// Strip receiver prefix if present.
	if idx := strings.LastIndex(testName, "."); idx >= 0 {
		testName = testName[idx+1:]
	}

	// Strip Test/Benchmark/Example prefix.
	var rest string
	for _, prefix := range []string{"Test", "Benchmark", "Example"} {
		if strings.HasPrefix(testName, prefix) {
			rest = testName[len(prefix):]
			break
		}
	}
	if rest == "" {
		return nil
	}

	// TestMain is special.
	if rest == "Main" && strings.HasPrefix(testName, "Test") {
		return nil
	}

	// If rest contains underscore, try progressively shorter method names.
	// TestType_Method         → ["Type.Method", "Type"]
	// TestType_Method_subtest → ["Type.Method_subtest", "Type.Method", "Type"]
	if idx := strings.Index(rest, "_"); idx > 0 {
		prefix := rest[:idx]
		suffix := rest[idx+1:]
		var candidates []string
		for suffix != "" {
			candidates = append(candidates, prefix+"."+suffix)
			last := strings.LastIndex(suffix, "_")
			if last < 0 {
				break
			}
			suffix = suffix[:last]
		}
		candidates = append(candidates, prefix)
		return candidates
	}

	return []string{rest}
}

// firstSourceCall walks a test function's body and returns the first
// source-file function it calls, reusing resolveCallee for call resolution.
func firstSourceCall(
	pass *analysis.Pass,
	testFn *funcInfo,
	sourceObjSet map[types.Object]bool,
	objToFunc map[types.Object]*funcInfo,
) *funcInfo {
	if testFn.decl.Body == nil {
		return nil
	}
	var found *funcInfo
	ast.Inspect(testFn.decl.Body, func(n ast.Node) bool {
		if found != nil {
			return false
		}
		call, ok := n.(*ast.CallExpr)
		if !ok {
			return true
		}
		if fi := resolveCallee(pass, call, sourceObjSet, objToFunc); fi != nil {
			found = fi
			return false
		}
		return true
	})
	return found
}

// funcDisplayName returns a human-readable name for a function declaration.
// Methods include the receiver type: "RecvType.Method".
func funcDisplayName(fd *ast.FuncDecl) string {
	if fd.Recv == nil || len(fd.Recv.List) == 0 {
		return fd.Name.Name
	}
	recv := fd.Recv.List[0].Type
	return fmt.Sprintf("%s.%s", typeName(recv), fd.Name.Name)
}

// typeName extracts a short type name from a receiver expression,
// stripping pointer indirection.
func typeName(expr ast.Expr) string {
	switch t := expr.(type) {
	case *ast.StarExpr:
		return typeName(t.X)
	case *ast.Ident:
		return t.Name
	case *ast.IndexExpr:
		return typeName(t.X)
	case *ast.IndexListExpr:
		return typeName(t.X)
	default:
		return "?"
	}
}

// isTestFunc returns true for Test*, Benchmark*, Example*, and TestMain.
func isTestFunc(name string) bool {
	// Strip receiver prefix if present.
	if idx := strings.LastIndex(name, "."); idx >= 0 {
		name = name[idx+1:]
	}
	return strings.HasPrefix(name, "Test") ||
		strings.HasPrefix(name, "Benchmark") ||
		strings.HasPrefix(name, "Example")
}

// isMethod returns true if the function declaration has a receiver (struct method).
func isMethod(fi *funcInfo) bool {
	return fi.decl.Recv != nil && len(fi.decl.Recv.List) > 0
}

// isUnexported returns true if the function name starts with a lowercase letter.
func isUnexported(name string) bool {
	// Strip receiver prefix if present.
	if idx := strings.LastIndex(name, "."); idx >= 0 {
		name = name[idx+1:]
	}
	if len(name) == 0 {
		return false
	}
	return name[0] >= 'a' && name[0] <= 'z'
}
