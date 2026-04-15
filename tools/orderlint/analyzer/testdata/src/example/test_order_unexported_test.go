package example

import "testing"

// TestParseBar tests parseBar (line 8) but appears before TestParseFoo
// which tests parseFoo (line 6). Since parseFoo is defined before parseBar,
// TestParseFoo should appear first.
func TestParseBar(t *testing.T) { parseBar() } // want `TestParseBar \(line 8\) should appear after TestParseFoo \(line 13\) — source function parseFoo \(line 6\) is defined before parseBar \(line 8\)`

// TestParseBaz has no matching source function — should be ignored.
func TestParseBaz(t *testing.T) {}

func TestParseFoo(t *testing.T) { parseFoo() }
