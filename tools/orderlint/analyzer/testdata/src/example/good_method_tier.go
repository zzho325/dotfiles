package example

// Methods before standalone helpers — no sibling violation even though
// the standalone helper (tierHelper) is called before the method (tierMethod)
// within TierEntry.

type tier struct{}

func TierEntry(t *tier) {
	tierHelper()
	t.tierMethod()
}

// Method comes after its caller but before standalone helpers — correct.
func (t *tier) tierMethod() {}

// Standalone helper is below all methods — correct tier ordering.
func tierHelper() {}
