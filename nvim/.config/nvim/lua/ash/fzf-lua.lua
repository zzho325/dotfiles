require("fzf-lua").setup({
	fzf_opts = {
		["--ansi"]     = "",
		["--multi"]    = "",
		["--extended"] = "",
	},
	keymap = {
		fzf = {
			["ctrl-q"] = "select-all+accept",
		}
	},
	fzf_colors = {
		-- use your CursorLine background for the selected row
		["bg+"]     = { "bg", "CursorLine" },
		-- make the selected text a contrasting color
		["fg+"]     = { "fg", "PmenuSel" },
		-- keep pointer (the bar) bold/bright
		["pointer"] = { "fg", "Conditional" },
	},
})
