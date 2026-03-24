package example

// Entry point — calls helper1 and helper2.
func EntryPoint() {
	helper1()
	helper2()
}

func helper1() {
	helper3()
}

func helper2() {}

func helper3() {}

// Another entry point, uncalled.
func AnotherEntry() {
	helper2()
}
