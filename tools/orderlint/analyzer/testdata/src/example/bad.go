package example

func calledBeforeCaller() {} // want `calledBeforeCaller \(line 3\) should appear after CallerFunc \(line 7\) — CallerFunc calls calledBeforeCaller`

type parser struct{}

func CallerFunc() {
	calledBeforeCaller()
}

func (p *parser) method() { // want `parser.method \(line 11\) should appear after parser.entry \(line 15\) — parser.entry calls parser.method`
	innerHelper()
}

func (p *parser) entry() {
	p.method()
}

// innerHelper is at line 19, called by parser.method at line 11. 19 > 11 = no violation.
func innerHelper() {}

// multiCaller: both A and B call shared. shared appears before B but after A — no violation.
func multiA() {
	multiShared()
}

func multiShared() {}

func multiB() {
	multiShared()
}
