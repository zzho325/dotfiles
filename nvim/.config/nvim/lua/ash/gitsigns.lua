require("gitsigns").setup({
	on_attach = function(bufnr)
		local gs = require("gitsigns")
		vim.keymap.set("n", "]c", function() gs.nav_hunk("next") end, { buffer = bufnr })
		vim.keymap.set("n", "[c", function() gs.nav_hunk("prev") end, { buffer = bufnr })
		vim.keymap.set("n", "<leader>hp", gs.preview_hunk, { buffer = bufnr })
		vim.keymap.set("n", "<leader>hr", gs.reset_hunk, { buffer = bufnr })
	end,
})
