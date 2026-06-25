package analyzer_test

import (
	"testing"

	"orderlint/analyzer"

	"golang.org/x/tools/go/analysis/analysistest"
)

func TestOrderLint(t *testing.T) {
	testdata := analysistest.TestData()
	analysistest.Run(t, testdata, analyzer.Analyzer, "example")
}
