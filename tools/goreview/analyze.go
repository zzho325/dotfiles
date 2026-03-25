package main

import (
	"fmt"
	"go/ast"
	"go/token"
	"go/types"
	"os"
	"sort"
	"strings"

	"golang.org/x/tools/go/packages"
)

// funcNode represents a function or method declaration.
type funcNode struct {
	name      string       // "Foo" or "RecvType.Method"
	recv      string       // receiver type name, empty for standalone functions
	shortName string       // just the func/method name
	sig       string       // "(ctx context.Context) → (string, error)"
	exported  bool         // starts with uppercase
	file      string       // absolute path
	line      int          // start line
	endLine   int          // end line (for diff overlap)
	obj       types.Object // resolved type object
}

// callChild pairs a callee with the line where it's called from its parent.
type callChild struct {
	fn       *funcNode
	callLine int // source line of the call expression in the caller
}

// callGraph holds a package's functions and their call relationships.
type callGraph struct {
	funcs   []*funcNode
	adj     map[*funcNode][]callChild // caller → callees, ordered by call-site line
	pkgPath string
}

func loadAndAnalyze(patterns []string) (*callGraph, error) {
	fset := token.NewFileSet()
	cfg := &packages.Config{
		Mode: packages.NeedSyntax | packages.NeedTypes |
			packages.NeedTypesInfo | packages.NeedFiles | packages.NeedName,
		Fset: fset,
	}
	pkgs, err := packages.Load(cfg, patterns...)
	if err != nil {
		return nil, err
	}
	for _, pkg := range pkgs {
		for _, e := range pkg.Errors {
			fmt.Fprintf(os.Stderr, "warning: %v\n", e)
		}
	}

	g := &callGraph{adj: map[*funcNode][]callChild{}}
	objToNode := map[types.Object]*funcNode{}
	fileInfo := map[string]*types.Info{}
	// Parallel slice: decls[i] is the AST for g.funcs[i].
	var decls []*ast.FuncDecl

	// Pass 1: discover all functions across all packages.
	for _, pkg := range pkgs {
		if g.pkgPath == "" {
			g.pkgPath = pkg.PkgPath
		}
		for _, file := range pkg.Syntax {
			fname := fset.Position(file.Pos()).Filename
			fileInfo[fname] = pkg.TypesInfo
			for _, d := range file.Decls {
				fd, ok := d.(*ast.FuncDecl)
				if !ok || fd.Name == nil {
					continue
				}
				obj := pkg.TypesInfo.Defs[fd.Name]
				if obj == nil {
					continue
				}
				fn := &funcNode{
					name:      displayName(fd),
					recv:      recvType(fd),
					shortName: fd.Name.Name,
					sig:       formatSig(obj, pkg.Types),
					exported:  fd.Name.IsExported(),
					file:      fname,
					line:      fset.Position(fd.Pos()).Line,
					endLine:   fset.Position(fd.End()).Line,
					obj:       obj,
				}
				g.funcs = append(g.funcs, fn)
				objToNode[obj] = fn
				decls = append(decls, fd)
			}
		}
	}

	// Pass 2: build cross-file call graph within the package.
	// Not deduplicated — repeat calls show with ↩ in the tree.
	for i, fn := range g.funcs {
		fd := decls[i]
		if fd.Body == nil {
			continue
		}
		info := fileInfo[fn.file]
		if info == nil {
			continue
		}
		ast.Inspect(fd.Body, func(n ast.Node) bool {
			call, ok := n.(*ast.CallExpr)
			if !ok {
				return true
			}
			callee := resolveCallee(info, call, objToNode)
			if callee == nil || callee == fn {
				return true
			}
			g.adj[fn] = append(g.adj[fn], callChild{
				fn:       callee,
				callLine: fset.Position(call.Pos()).Line,
			})
			return true
		})
	}

	// Sort children by call-site line.
	for _, children := range g.adj {
		sort.Slice(children, func(i, j int) bool {
			return children[i].callLine < children[j].callLine
		})
	}

	return g, nil
}

// resolveCallee resolves a call expression to a funcNode if it's in our graph.
func resolveCallee(
	info *types.Info,
	call *ast.CallExpr,
	objToNode map[types.Object]*funcNode,
) *funcNode {
	var ident *ast.Ident
	switch fn := call.Fun.(type) {
	case *ast.Ident:
		ident = fn
	case *ast.SelectorExpr:
		ident = fn.Sel
	default:
		return nil
	}
	obj := info.Uses[ident]
	if obj == nil {
		return nil
	}
	return objToNode[obj]
}

func displayName(fd *ast.FuncDecl) string {
	if fd.Recv == nil || len(fd.Recv.List) == 0 {
		return fd.Name.Name
	}
	return fmt.Sprintf("%s.%s", astTypeName(fd.Recv.List[0].Type), fd.Name.Name)
}

func recvType(fd *ast.FuncDecl) string {
	if fd.Recv == nil || len(fd.Recv.List) == 0 {
		return ""
	}
	return astTypeName(fd.Recv.List[0].Type)
}

func astTypeName(expr ast.Expr) string {
	switch t := expr.(type) {
	case *ast.StarExpr:
		return astTypeName(t.X)
	case *ast.Ident:
		return t.Name
	case *ast.IndexExpr:
		return astTypeName(t.X)
	case *ast.IndexListExpr:
		return astTypeName(t.X)
	default:
		return "?"
	}
}

// formatSig renders a function signature as "(params) → returns".
func formatSig(obj types.Object, pkg *types.Package) string {
	sig, ok := obj.Type().(*types.Signature)
	if !ok {
		return ""
	}
	// Use short package names for external types (rsa.PrivateKey, not crypto/rsa.PrivateKey).
	q := func(other *types.Package) string {
		if other == pkg {
			return ""
		}
		return other.Name()
	}

	params := sig.Params()
	var ps []string
	for i := range params.Len() {
		p := params.At(i)
		ts := types.TypeString(p.Type(), q)
		if p.Name() != "" {
			ps = append(ps, p.Name()+" "+ts)
		} else {
			ps = append(ps, ts)
		}
	}

	results := sig.Results()
	if results.Len() == 0 {
		if len(ps) == 0 {
			return "()"
		}
		return "(" + strings.Join(ps, ", ") + ")"
	}

	var rs []string
	for i := range results.Len() {
		rs = append(rs, types.TypeString(results.At(i).Type(), q))
	}
	ret := strings.Join(rs, ", ")
	if results.Len() > 1 {
		ret = "(" + ret + ")"
	}
	return "(" + strings.Join(ps, ", ") + ") " + ret
}
