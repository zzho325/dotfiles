package example

// Contradictory sibling orderings across callers, including nested calls.
//   ContradictRecords calls contribBatch (nested) → contribInsert → contribRecord
//   ContradictSender calls contribRecord → contribBatch
// The first call-site (contribBatch, nested inside contribInsert) wins.

func ContradictRecords() {
	contribInsert(contribBatch())
	contribRecord()
}

func ContradictSender() {
	contribRecord()
	contribBatch()
}

func contribBatch() int { return 0 }

func contribInsert(_ int) {}

func contribRecord() {}
