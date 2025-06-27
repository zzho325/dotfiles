require("gruvbox").setup({ contrast = "hard" })

require('auto-dark-mode').setup({
  -- how often to poll (in ms)
  update_interval = 3000,
  -- what to do when switching *to* dark
  set_dark_mode = function()
    vim.opt.background = 'dark'
	vim.cmd('colorscheme gruvbox')
  end,
  -- what to do when switching *to* lightset_light_mode
  set_light_mode = function() 
	vim.opt.background = 'light'
	vim.cmd('colorscheme gruvbox')
  end,
  -- fallback if detection fails
  fallback = 'dark',
})
