if vim.env.SSH_TTY then
	vim.opt.clipboard:append('unnamedplus')

	vim.g.clipboard = {
		name = 'OSC 52',
		copy = {
			['+'] = require('vim.ui.clipboard.osc52').copy('+'),
			['*'] = require('vim.ui.clipboard.osc52').copy('*'),
		},
		paste = {
			['+'] = function () end, 
			['*'] = function () end,
		},
	}
end
