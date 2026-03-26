package analyzer

import (
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"strings"

	"golang.org/x/tools/go/analysis"
	"golang.org/x/tools/go/analysis/passes/inspect"
)

var graphFlag bool

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
		analyzeFile(pass, fileName, funcs, objToFunc, isTest)
	}

	return nil, nil
}

// analyzeFile checks function ordering within a single file.
func analyzeFile(
	pass *analysis.Pass,
	fileName string,
	funcs []*funcInfo,
	objToFunc map[types.Object]*funcInfo,
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

	// Test file: check that helpers appear after test functions.
	if isTest {
		checkTestHelperOrdering(pass, funcs)
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
	for _, fi := range funcs {
		if !isTestFunc(fi.name) && isUnexported(fi.name) && fi.line < lastTestLine {
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
