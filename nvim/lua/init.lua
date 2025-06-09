require "ash.options"
require "ash.keymaps"
require "ash.plugins"
require "ash.cmp"
require "ash.lsp"
-- require "ash.autosave"

vim.o.background = "light" -- or "light" for light mode
-- Setup gruvbox with options
require("gruvbox").setup({
  contrast = "hard",
  terminal_colors = true,
  undercurl = true,
  underline = true,
  bold = true,
  italic = {
    strings = true,
    emphasis = true,
    comments = true,
    operators = false,
    folds = true,
  },
  strikethrough = true,
  invert_selection = false,
  invert_signs = false,
  invert_tabline = false,
  inverse = true,
  palette_overrides = {},
  overrides = {},
  dim_inactive = false,
  transparent_mode = false,
})
vim.cmd([[colorscheme gruvbox]])
