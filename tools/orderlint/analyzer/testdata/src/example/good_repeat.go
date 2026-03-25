package example

func RepeatCaller() {
	repeatA()
	repeatB()
	repeatA() // called again — first call determines order
}

func repeatA() {}

func repeatB() {}
