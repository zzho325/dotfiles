-- In a jj repo, diff against @- so gutter signs show the current change.
local function jj_base()
	local ok, out = pcall(vim.fn.systemlist, "jj log -r @- --no-graph -T commit_id --limit 1")
	if ok and vim.v.shell_error == 0 and out[1] and #out[1] > 0 then
		return out[1]
	end
	return nil
end

-- Refresh gitsigns base when the buffer is re-entered (e.g. after a jj command).
local jj_group = vim.api.nvim_create_augroup("JjGitsigns", { clear = true })
vim.api.nvim_create_autocmd("BufEnter", {
	group = jj_group,
	callback = function(ev)
		local gs = package.loaded["gitsigns"]
		if not gs then return end
		local base = jj_base()
		if base then
			gs.change_base(base, ev.buf)
		end
	end,
})

require("gitsigns").setup({
	on_attach = function(bufnr)
		local gs = require("gitsigns")

		-- Set jj base on first attach.
		local base = jj_base()
		if base then
			gs.change_base(base, bufnr)
		end

		vim.keymap.set("n", "]c", function() gs.nav_hunk("next") end, { buffer = bufnr })
		vim.keymap.set("n", "[c", function() gs.nav_hunk("prev") end, { buffer = bufnr })
		vim.keymap.set("n", "<leader>hp", gs.preview_hunk, { buffer = bufnr })
		vim.keymap.set("n", "<leader>hr", gs.reset_hunk, { buffer = bufnr })
	end,
})
