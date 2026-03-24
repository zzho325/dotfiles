package example

import "testing"

func testHelper() {} // want `testHelper \(line 5\) should appear after TestFirst \(line 7\) — TestFirst calls testHelper` `test helper testHelper \(line 5\) should appear after all test functions \(last test at line 11\)`

func TestFirst(t *testing.T) {
	testHelper()
}

func TestSecond(t *testing.T) {
	testHelper()
}
