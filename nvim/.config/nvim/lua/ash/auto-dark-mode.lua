require("rose-pine").setup({
	variant      = "auto", -- automatically choose based on background
	dark_variant = "main", -- use “moon” when background = dark
})

-- apply once for initial colorscheme
vim.cmd("colorscheme rose-pine")

require("auto-dark-mode").setup({
	update_interval = 3000,

	set_dark_mode = function()
		vim.opt.background = "dark"
		vim.cmd("colorscheme rose-pine")
	end,

	set_light_mode = function()
		vim.opt.background = "light"
		vim.cmd("colorscheme rose-pine")
	end,

	fallback = "dark",
})
