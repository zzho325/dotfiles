package example

func SiblingCaller() {
	siblingB()
	siblingA()
}

func siblingA() {}

func siblingB() {} // want `siblingB .* should appear before siblingA .* SiblingCaller calls siblingB .* before siblingA`
