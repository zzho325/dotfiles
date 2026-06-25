package example

// First entry point.
func FirstEntry() {
	firstHelper()
}

// Second entry point.
func SecondEntry() {
	secondHelper()
}

func secondHelper() {} // want `secondHelper .* should appear after firstHelper .* firstHelper is reached before secondHelper in the call tree`


func firstHelper() {}
