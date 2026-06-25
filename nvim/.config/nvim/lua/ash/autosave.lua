vim.api.nvim_create_autocmd({ "InsertLeave", "BufLeave", "FocusLost", "TextChanged" }, {
	pattern = "*",
	callback = function()
		if vim.bo.modifiable and vim.bo.modified and vim.bo.buftype == "" then
			vim.cmd("silent write")
		end
	end,
})
