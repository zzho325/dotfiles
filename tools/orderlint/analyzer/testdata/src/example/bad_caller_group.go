package example

// First entry point.
func FirstEntry() {
	firstHelper()
}

// Second entry point.
func SecondEntry() {
	secondHelper()
}

func secondHelper() {} // want `secondHelper .* should appear after firstHelper .* firstHelper's caller FirstEntry .* appears before secondHelper's caller SecondEntry`


func firstHelper() {}
