package example

func BadRepeatCaller() {
	badRepeatB()
	badRepeatA()
	badRepeatB() // second call — first call determines order
}

func badRepeatA() {}

func badRepeatB() {} // want `badRepeatB .* should appear before badRepeatA .* BadRepeatCaller calls badRepeatB .* before badRepeatA`
