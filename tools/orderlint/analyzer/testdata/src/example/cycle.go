package example

func cycleA() { // want `cycle detected: cycleA → cycleB → cycleA`
	cycleB()
}

func cycleB() {
	cycleA()
}
