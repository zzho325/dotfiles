package example

// Source file with unexported functions.
// Name-based matching won't work (TestParseBar → "ParseBar" != "parseBar").

func parseFoo() {}

func parseBar() {}
