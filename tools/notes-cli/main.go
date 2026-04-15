package main

import (
	"bufio"
	"fmt"
	"os"
	"regexp"
	"strconv"
	"strings"
)

func main() {
	if len(os.Args) < 2 {
		printUsage()
		os.Exit(1)
	}

	// Support -f <file> to override the default notes.md.
	args := os.Args[1:]
	customFile := ""
	if len(args) >= 2 && args[0] == "-f" {
		customFile = args[1]
		args = args[2:]
	}
	if len(args) == 0 {
		printUsage()
		os.Exit(1)
	}

	path := findNotesFile(customFile)
	if path == "" {
		fmt.Fprintln(os.Stderr, "notes file not found")
		os.Exit(1)
	}

	switch args[0] {
	case "wip":
		if len(args) > 1 {
			wipArgs, readStdin := consumeStdinFlag(args[1:])
			cmdWipAdd(path, strings.Join(wipArgs, " "), readStdin)
		} else {
			cmdWip(path)
		}

	case "reply":
		if len(args) < 2 {
			fmt.Fprintln(os.Stderr, "usage: notes reply <N> \"text\" [-]")
			os.Exit(1)
		}
		replyArgs, readStdin := consumeStdinFlag(args[2:])
		cmdReply(path, args[1], strings.Join(replyArgs, " "), readStdin)

	case "resolve":
		if len(args) < 2 {
			fmt.Fprintln(os.Stderr, "usage: notes resolve <N|all>")
			os.Exit(1)
		}
		cmdResolve(path, args[1])
	case "done":
		cmdDone(path)
	case "propose":
		if len(args) < 2 {
			fmt.Fprintln(os.Stderr, "usage: notes propose \"title\" [-b \"body\" | -]")
			os.Exit(1)
		}
		body, propArgs := extractBodyFlag(args[1:])
		propArgs, readStdin := consumeStdinFlag(propArgs)
		cmdPropose(path, strings.Join(propArgs, " "), body, readStdin)
	case "proposals":
		cmdProposals(path)
	case "update":
		if len(args) < 3 {
			fmt.Fprintln(os.Stderr, "usage: notes update <N> \"desc\" [-b \"body\" | -]")
			os.Exit(1)
		}
		body, updateArgs := extractBodyFlag(args[2:])
		updateArgs, readStdin := consumeStdinFlag(updateArgs)
		cmdUpdate(path, args[1], strings.Join(updateArgs, " "), body, readStdin)

	case "stamp":
		if len(args) < 2 {
			fmt.Fprintln(os.Stderr, "usage: notes stamp <N>")
			os.Exit(1)
		}
		cmdStamp(path, args[1])
	case "approved":
		cmdApproved(path)
	case "delete":
		if len(args) < 2 {
			fmt.Fprintln(os.Stderr, "usage: notes delete <N>")
			os.Exit(1)
		}
		cmdDelete(path, args[1])
	case "applied":
		cmdApplied(path)
	default:
		printUsage()
		os.Exit(1)
	}
}

func printUsage() {
	fmt.Fprintln(os.Stderr, `usage: notes [-f <file>] <command>

threads:
  wip                    list WIP threads
  wip "title" [-]        add WIP thread (pipe body via stdin)
  reply <N> "text" [-]   append to WIP thread N
  resolve <N>            move WIP thread N to Done
  resolve all            move all WIP threads to Done
  done                   list Done thread summaries

proposals:
  propose "desc" [-b "body" | -]   add proposal
  update <N> "desc" [-b "body" | -]  update proposal N
  proposals              list all proposals
  stamp <N>              mark proposal N as approved [x]
  delete <N>             delete proposal N
  approved               list only approved [x] proposals
  applied                move approved [x] proposals to Done`)
}

// stdinIsPipe returns true if stdin is a pipe (not a terminal).
func stdinIsPipe() bool {
	fi, err := os.Stdin.Stat()
	if err != nil {
		return false
	}
	return fi.Mode()&os.ModeCharDevice == 0
}

// consumeStdinFlag checks for a trailing "-" arg (explicit stdin) or a piped
// stdin. Returns the remaining args and whether to read stdin.
func consumeStdinFlag(args []string) ([]string, bool) {
	if len(args) > 0 && args[len(args)-1] == "-" {
		return args[:len(args)-1], true
	}
	return args, stdinIsPipe()
}

// extractBodyFlag extracts a "-b <value>" flag from args, returning the value
// and the remaining args. Returns empty string if not present.
func extractBodyFlag(args []string) (string, []string) {
	for i := 0; i < len(args)-1; i++ {
		if args[i] == "-b" {
			return args[i+1], append(args[:i], args[i+2:]...)
		}
	}
	return "", args
}

// readIndentedStdin reads all lines from stdin, prepending indent to
// non-empty lines.
func readIndentedStdin(indent string) []string {
	var lines []string
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for scanner.Scan() {
		line := scanner.Text()
		if line == "" {
			lines = append(lines, "")
		} else {
			lines = append(lines, indent+line)
		}
	}
	return lines
}

// indentLines splits text on newlines and prepends indent to non-empty lines.
func indentLines(text, indent string) []string {
	var lines []string
	for _, line := range strings.Split(text, "\n") {
		if line == "" {
			lines = append(lines, "")
		} else {
			lines = append(lines, indent+line)
		}
	}
	return lines
}

func findNotesFile(custom string) string {
	if custom != "" {
		if _, err := os.Stat(custom); err == nil {
			return custom
		}
		return ""
	}
	if _, err := os.Stat("notes.md"); err == nil {
		return "notes.md"
	}
	return ""
}

// ── Types ──

// thread is a parsed WIP thread.
type thread struct {
	num   int
	lines []string
}

func (t thread) firstLine() string {
	if len(t.lines) == 0 {
		return ""
	}
	re := regexp.MustCompile(`^\d+\.\s*`)
	return re.ReplaceAllString(t.lines[0], "")
}

func (t thread) summary() string {
	s := t.firstLine()
	if len(s) > 80 {
		s = s[:77] + "..."
	}
	return s
}

func (t thread) body() string {
	return strings.Join(t.lines, "\n")
}

// proposal is a parsed proposed change.
type proposal struct {
	num     int
	stamped bool
	lines   []string
}

func (pr proposal) description() string {
	re := regexp.MustCompile(`^-\s*\[.\]\s*\*\*P\d+\*\*\s*`)
	return re.ReplaceAllString(pr.lines[0], "")
}

// propChunk is either a proposal or raw text in the proposals section.
// Preserves interleaved content (verification blocks, PR descriptions)
// in its original position.
type propChunk struct {
	proposal *proposal // non-nil for proposal chunks
	raw      string    // non-empty for raw text chunks
}

// parsedFile holds the parsed structure of notes.md.
type parsedFile struct {
	beforeWIP  string
	wipHeader  string
	threads    []thread
	middle     string // between WIP and Proposed changes
	propHeader string
	propChunks []propChunk
	doneStart  string
	doneBody   string
}

// ── Parsing ──

func parseFile(path string) (*parsedFile, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	content := string(data)
	p := &parsedFile{}

	wipIdx := strings.Index(content, "### WIP")
	if wipIdx == -1 {
		return nil, fmt.Errorf("### WIP section not found")
	}
	p.beforeWIP = content[:wipIdx]
	rest := content[wipIdx:]

	propIdx := strings.Index(rest, "### Proposed changes")
	if propIdx == -1 {
		propIdx = strings.Index(rest, "### Proposed Changes")
	}

	doneIdx := strings.Index(rest, "### Done")
	if doneIdx == -1 {
		return nil, fmt.Errorf("### Done section not found")
	}

	if propIdx != -1 && propIdx < doneIdx {
		parseWipSection(p, rest[:propIdx])
		parsePropSection(p, rest[propIdx:doneIdx])
		parseDoneSection(p, rest[doneIdx:])
	} else {
		parseWipSection(p, rest[:doneIdx])
		parseDoneSection(p, rest[doneIdx:])
	}

	return p, nil
}

func parseWipSection(p *parsedFile, section string) {
	lines := strings.Split(section, "\n")
	p.wipHeader = lines[0] + "\n"

	// Strip placeholder "(none)" lines
	var filtered []string
	filtered = append(filtered, lines[0])
	for _, l := range lines[1:] {
		if strings.TrimSpace(l) != "(none)" {
			filtered = append(filtered, l)
		}
	}
	lines = filtered

	itemRe := regexp.MustCompile(`^(\d+)\.\s`)

	type span struct {
		num   int
		start int
		end   int
	}
	var spans []span

	for i := 1; i < len(lines); i++ {
		m := itemRe.FindStringSubmatch(lines[i])
		if m != nil {
			n, _ := strconv.Atoi(m[1])
			if len(spans) > 0 {
				spans[len(spans)-1].end = i
			}
			spans = append(spans, span{num: n, start: i})
		}
	}

	afterThreads := len(lines)
	if len(spans) > 0 {
		lastStart := spans[len(spans)-1].start
		for i := lastStart + 1; i < len(lines); i++ {
			if strings.HasPrefix(lines[i], "### ") ||
				strings.HasPrefix(lines[i], "**PR ") {
				spans[len(spans)-1].end = i
				afterThreads = i
				break
			}
		}
		if spans[len(spans)-1].end == 0 {
			spans[len(spans)-1].end = len(lines)
			afterThreads = len(lines)
		}
	}

	if len(spans) == 0 {
		mid := strings.Join(lines[1:], "\n")
		if strings.TrimSpace(mid) != "" {
			p.middle = mid
		}
		return
	}

	for _, s := range spans {
		end := s.end
		if end == 0 {
			end = afterThreads
		}
		for end > s.start && strings.TrimSpace(lines[end-1]) == "" {
			end--
		}
		threadLines := make([]string, end-s.start)
		copy(threadLines, lines[s.start:end])
		p.threads = append(p.threads, thread{
			num:   s.num,
			lines: threadLines,
		})
	}

	if afterThreads < len(lines) {
		p.middle = strings.Join(lines[afterThreads:], "\n")
	}
}

// parsePropSection parses the Proposed changes section into interleaved
// chunks of proposals and raw text. This preserves free-form content
// (verification blocks, PR descriptions, separators) in place.
func parsePropSection(p *parsedFile, section string) {
	lines := strings.Split(section, "\n")

	// Strip placeholder "(none pending)" lines
	var filtered []string
	for _, l := range lines {
		if strings.TrimSpace(l) != "(none pending)" {
			filtered = append(filtered, l)
		}
	}
	lines = filtered

	p.propHeader = lines[0] + "\n"

	propRe := regexp.MustCompile(`^-\s*\[([ x])\]\s*\*\*P(\d+)\*\*`)

	var currentProp *proposal
	var currentRaw []string
	inFence := false

	flushRaw := func() {
		if len(currentRaw) > 0 {
			p.propChunks = append(p.propChunks, propChunk{
				raw: strings.Join(currentRaw, "\n"),
			})
			currentRaw = nil
		}
	}

	flushProp := func() {
		if currentProp != nil {
			// Trim trailing blank lines
			for len(currentProp.lines) > 0 &&
				strings.TrimSpace(currentProp.lines[len(currentProp.lines)-1]) == "" {
				currentProp.lines = currentProp.lines[:len(currentProp.lines)-1]
			}
			p.propChunks = append(p.propChunks, propChunk{
				proposal: currentProp,
			})
			currentProp = nil
		}
	}

	for i := 1; i < len(lines); i++ {
		line := lines[i]
		trimmed := strings.TrimSpace(line)

		// Track code fences inside proposals
		if strings.HasPrefix(trimmed, "```") && currentProp != nil {
			inFence = !inFence
			currentProp.lines = append(currentProp.lines, line)
			continue
		}
		if inFence && currentProp != nil {
			currentProp.lines = append(currentProp.lines, line)
			continue
		}

		m := propRe.FindStringSubmatch(line)
		if m != nil {
			flushRaw()
			flushProp()
			n, _ := strconv.Atoi(m[2])
			currentProp = &proposal{
				num:     n,
				stamped: m[1] == "x",
				lines:   []string{line},
			}
		} else if currentProp != nil &&
			(strings.HasPrefix(line, "  ") || trimmed == "") {
			currentProp.lines = append(currentProp.lines, line)
		} else {
			flushProp()
			currentRaw = append(currentRaw, line)
		}
	}
	flushProp()
	flushRaw()
}

func parseDoneSection(p *parsedFile, section string) {
	doneLines := strings.SplitN(section, "\n", 2)
	p.doneStart = doneLines[0] + "\n"
	if len(doneLines) > 1 {
		p.doneBody = doneLines[1]
	}
}

// ── Write back ──

func (p *parsedFile) writeBack(path string) error {
	var b strings.Builder
	b.WriteString(p.beforeWIP)
	b.WriteString(p.wipHeader)
	b.WriteString("\n")

	if len(p.threads) == 0 {
		b.WriteString("(none)\n")
	} else {
		for _, t := range p.threads {
			b.WriteString(strings.Join(t.lines, "\n"))
			b.WriteString("\n\n")
		}
	}

	if p.middle != "" {
		b.WriteString(p.middle)
		if !strings.HasSuffix(p.middle, "\n") {
			b.WriteString("\n")
		}
	}

	if p.propHeader != "" {
		b.WriteString(p.propHeader)
		for _, chunk := range p.propChunks {
			if chunk.proposal != nil {
				b.WriteString(strings.Join(chunk.proposal.lines, "\n"))
				b.WriteString("\n")
			} else {
				b.WriteString(chunk.raw)
				if !strings.HasSuffix(chunk.raw, "\n") {
					b.WriteString("\n")
				}
			}
		}
	}

	b.WriteString(p.doneStart)
	b.WriteString(p.doneBody)

	return os.WriteFile(path, []byte(b.String()), 0644)
}

// ── Helpers ──

func (p *parsedFile) nextThreadNum() int {
	max := 0
	for _, t := range p.threads {
		if t.num > max {
			max = t.num
		}
	}
	return max + 1
}

func (p *parsedFile) nextPropNum() int {
	max := 0
	for _, chunk := range p.propChunks {
		if chunk.proposal != nil && chunk.proposal.num > max {
			max = chunk.proposal.num
		}
	}
	return max + 1
}

// addProposal inserts a proposal after the last existing proposal,
// or at the start of chunks if no proposals exist yet.
func (p *parsedFile) addProposal(pr proposal) {
	chunk := propChunk{proposal: &pr}
	lastPropIdx := -1
	for i, c := range p.propChunks {
		if c.proposal != nil {
			lastPropIdx = i
		}
	}
	if lastPropIdx >= 0 {
		tail := make([]propChunk, len(p.propChunks[lastPropIdx+1:]))
		copy(tail, p.propChunks[lastPropIdx+1:])
		p.propChunks = append(
			append(p.propChunks[:lastPropIdx+1], chunk), tail...,
		)
	} else {
		p.propChunks = append([]propChunk{chunk}, p.propChunks...)
	}
}

func (p *parsedFile) allProposals() []proposal {
	var result []proposal
	for _, chunk := range p.propChunks {
		if chunk.proposal != nil {
			result = append(result, *chunk.proposal)
		}
	}
	return result
}

func appendDone(doneBody string, summary string, body string) string {
	detail := fmt.Sprintf(
		"\n<details><summary>Resolved: %s</summary>\n\n%s\n\n</details>\n",
		summary, body,
	)
	return detail + doneBody
}

// ── Commands ──

func cmdWip(path string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
	if len(p.threads) == 0 {
		fmt.Println("(no WIP threads)")
		return
	}
	for i, t := range p.threads {
		for _, line := range t.lines {
			fmt.Println(line)
		}
		if i < len(p.threads)-1 {
			fmt.Println()
		}
	}
}

func cmdWipAdd(path string, title string, readStdin bool) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	n := p.nextThreadNum()
	threadLines := []string{fmt.Sprintf("%d. %s", n, title)}

	if readStdin {
		threadLines = append(threadLines, "")
		threadLines = append(threadLines, readIndentedStdin("   ")...)
	}

	p.threads = append(p.threads, thread{
		num:   n,
		lines: threadLines,
	})

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("added: %d. %s\n", n, title)
}

func cmdReply(path string, arg string, text string, readStdin bool) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	n, err := strconv.Atoi(arg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid thread number: %s\n", arg)
		os.Exit(1)
	}

	idx := -1
	for i, t := range p.threads {
		if t.num == n {
			idx = i
			break
		}
	}
	if idx == -1 {
		fmt.Fprintf(os.Stderr, "WIP thread %d not found\n", n)
		os.Exit(1)
	}

	t := &p.threads[idx]
	t.lines = append(t.lines, "")

	if text != "" {
		t.lines = append(t.lines, "   "+text)
	}

	if readStdin {
		t.lines = append(t.lines, readIndentedStdin("   ")...)
	}

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("replied to: %d. %s\n", n, t.firstLine())
}

func cmdResolve(path string, arg string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	if arg == "all" {
		if len(p.threads) == 0 {
			fmt.Println("no WIP threads to resolve")
			return
		}
		for _, t := range p.threads {
			p.doneBody = appendDone(p.doneBody, t.summary(), t.body())
			fmt.Printf("resolved: %d. %s\n", t.num, t.firstLine())
		}
		p.threads = nil
		if err := p.writeBack(path); err != nil {
			fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
			os.Exit(1)
		}
		return
	}

	n, err := strconv.Atoi(arg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid thread number: %s\n", arg)
		os.Exit(1)
	}

	idx := -1
	for i, t := range p.threads {
		if t.num == n {
			idx = i
			break
		}
	}
	if idx == -1 {
		fmt.Fprintf(os.Stderr, "WIP thread %d not found\n", n)
		os.Exit(1)
	}

	t := p.threads[idx]
	p.threads = append(p.threads[:idx], p.threads[idx+1:]...)
	p.doneBody = appendDone(p.doneBody, t.summary(), t.body())

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("resolved: %d. %s\n", n, t.firstLine())
}

func cmdDone(path string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
	re := regexp.MustCompile(`<summary>(.*?)</summary>`)
	matches := re.FindAllStringSubmatch(p.doneBody, -1)
	if len(matches) == 0 {
		fmt.Println("(no Done threads)")
		return
	}
	for _, m := range matches {
		fmt.Printf("  - %s\n", m[1])
	}
}

func cmdPropose(path string, desc string, body string, readStdin bool) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	if p.propHeader == "" {
		p.propHeader = "### Proposed changes\n"
	}

	n := p.nextPropNum()
	titleLine := fmt.Sprintf("- [ ] **P%d** %s", n, desc)
	propLines := []string{titleLine}

	if body != "" {
		propLines = append(propLines, indentLines(body, "  ")...)
	}
	if readStdin {
		propLines = append(propLines, readIndentedStdin("  ")...)
	}

	pr := proposal{
		num:     n,
		stamped: false,
		lines:   propLines,
	}
	p.addProposal(pr)

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("proposed: P%d %s\n", n, desc)
}

func cmdUpdate(path string, arg string, desc string, body string, readStdin bool) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	n, err := strconv.Atoi(arg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid proposal number: %s\n", arg)
		os.Exit(1)
	}

	found := false
	for i := range p.propChunks {
		pr := p.propChunks[i].proposal
		if pr == nil || pr.num != n {
			continue
		}
		found = true

		checkbox := " "
		if pr.stamped {
			checkbox = "x"
		}
		titleLine := fmt.Sprintf("- [%s] **P%d** %s", checkbox, n, desc)

		hasNewBody := body != "" || readStdin
		if hasNewBody {
			propLines := []string{titleLine}
			if body != "" {
				propLines = append(propLines, indentLines(body, "  ")...)
			}
			if readStdin {
				propLines = append(propLines, readIndentedStdin("  ")...)
			}
			pr.lines = propLines
		} else {
			pr.lines[0] = titleLine
		}
		break
	}

	if !found {
		fmt.Fprintf(os.Stderr, "proposal P%d not found\n", n)
		os.Exit(1)
	}

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("updated: P%d %s\n", n, desc)
}

func cmdProposals(path string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
	proposals := p.allProposals()
	if len(proposals) == 0 {
		fmt.Println("(no proposals)")
		return
	}
	for _, pr := range proposals {
		for _, line := range pr.lines {
			fmt.Println(line)
		}
	}
}

func cmdStamp(path string, arg string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	n, err := strconv.Atoi(arg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid proposal number: %s\n", arg)
		os.Exit(1)
	}

	found := false
	for i := range p.propChunks {
		pr := p.propChunks[i].proposal
		if pr != nil && pr.num == n {
			pr.stamped = true
			pr.lines[0] = strings.Replace(pr.lines[0], "[ ]", "[x]", 1)
			found = true
			break
		}
	}
	if !found {
		fmt.Fprintf(os.Stderr, "proposal P%d not found\n", n)
		os.Exit(1)
	}

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("stamped: P%d\n", n)
}

func cmdDelete(path string, arg string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	n, err := strconv.Atoi(arg)
	if err != nil {
		fmt.Fprintf(os.Stderr, "invalid proposal number: %s\n", arg)
		os.Exit(1)
	}

	found := false
	var remaining []propChunk
	var desc string
	for _, chunk := range p.propChunks {
		if chunk.proposal != nil && chunk.proposal.num == n {
			found = true
			desc = chunk.proposal.description()
			continue
		}
		remaining = append(remaining, chunk)
	}
	if !found {
		fmt.Fprintf(os.Stderr, "proposal P%d not found\n", n)
		os.Exit(1)
	}

	p.propChunks = remaining

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("deleted: P%d %s\n", n, desc)
}

func cmdApproved(path string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}
	found := false
	for _, pr := range p.allProposals() {
		if pr.stamped {
			fmt.Printf("  P%d %s\n", pr.num, pr.description())
			found = true
		}
	}
	if !found {
		fmt.Println("(no approved proposals)")
	}
}

func cmdApplied(path string) {
	p, err := parseFile(path)
	if err != nil {
		fmt.Fprintf(os.Stderr, "error: %v\n", err)
		os.Exit(1)
	}

	var stamped []proposal
	var remaining []propChunk
	for _, chunk := range p.propChunks {
		if chunk.proposal != nil && chunk.proposal.stamped {
			stamped = append(stamped, *chunk.proposal)
		} else {
			remaining = append(remaining, chunk)
		}
	}

	if len(stamped) == 0 {
		fmt.Println("no stamped proposals to clear")
		return
	}

	var items []string
	for _, pr := range stamped {
		items = append(items, fmt.Sprintf(
			"- P%d: %s", pr.num, pr.description(),
		))
	}
	p.doneBody = appendDone(
		p.doneBody, "Applied proposals",
		strings.Join(items, "\n"),
	)
	p.propChunks = remaining

	if err := p.writeBack(path); err != nil {
		fmt.Fprintf(os.Stderr, "error writing: %v\n", err)
		os.Exit(1)
	}
	for _, pr := range stamped {
		fmt.Printf("applied: P%d %s\n", pr.num, pr.description())
	}
}
