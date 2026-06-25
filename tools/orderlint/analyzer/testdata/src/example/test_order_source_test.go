package example

import "testing"

// TestGamma appears first but Gamma is last in source.
func TestGamma(t *testing.T) {} // want `TestGamma \(line 6\) should appear after TestBeta \(line 9\) — source function Beta \(line 8\) is defined before Gamma \(line 10\)`

// TestBeta appears second but Beta is before Gamma — OK relative to Gamma above, but after Alpha below.
func TestBeta(t *testing.T) {} // want `TestBeta \(line 9\) should appear after TestAlpha \(line 12\) — source function Alpha \(line 6\) is defined before Beta \(line 8\)`

// TestAlpha appears last but Alpha is first in source.
func TestAlpha(t *testing.T) {}
