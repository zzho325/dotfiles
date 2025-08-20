require "ash.clipboard"
require "ash.options"
require "ash.keymaps"
require "ash.plugins"
require "ash.cmp"
require "ash.lsp"
require "ash.auto-dark-mode"
require "ash.nvim-tree"
require "ash.fzf-lua"

-- project level override
local proj_cfg = vim.fn.getcwd() .. "/.nvim.lua"
if vim.fn.filereadable(proj_cfg) == 1 then
	dofile(proj_cfg)
end

-- Set transparent highlights
vim.cmd [[
  augroup TransparentBG
    autocmd!
    autocmd ColorScheme * highlight Normal guibg=none
    autocmd ColorScheme * highlight NormalNC guibg=none
    autocmd ColorScheme * highlight NonText guibg=none
    autocmd ColorScheme * highlight LineNr guibg=none
    autocmd ColorScheme * highlight EndOfBuffer guibg=none
  augroup END
]]
