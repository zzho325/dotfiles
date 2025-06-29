local opts = { noremap = true, silent = true }
local term_opts = { silent = true }

-- Shorten function name
local keymap = vim.keymap.set

-- Remap space as leader key
keymap("", "<Space>", "<Nop>", opts)
vim.g.mapleader = " "
vim.g.maplocalleader = " "

-- Modes
--   normal_mode = "n",
--   insert_mode = "i",
--   visual_mode = "v",
--   visual_block_mode = "x",
--   term_mode = "t",
--   command_mode = "c",

-- Normal --
-- Better window navigation
keymap("n", "<C-h>", "<C-w>h", opts)
keymap("n", "<C-j>", "<C-w>j", opts)
keymap("n", "<C-k>", "<C-w>k", opts)
keymap("n", "<C-l>", "<C-w>l", opts)

-- Resize with arrows
keymap("n", "<C-Up>", ":resize +2<CR>", opts)
keymap("n", "<C-Down>", ":resize -2<CR>", opts)
keymap("n", "<C-Left>", ":vertical resize -2<CR>", opts)
keymap("n", "<C-Right>", ":vertical resize +2<CR>", opts)

-- Navigate buffers
keymap("n", "<S-l>", ":bnext<CR>", opts)
keymap("n", "<S-h>", ":bprevious<CR>", opts)

-- Insert --
-- Press jk fast to entervi
-- keymap("i", "jk", "<ESC>", opts)

-- Visual --
-- Stay in indent mode
keymap("v", "<", "<gv", opts)
keymap("v", ">", ">gv", opts)

-- Move text up and down
-- nnoremap <A-j> :m .+1<CR>==
-- nnoremap <A-k> :m .-2<CR>==
-- inoremap <A-j> <Esc>:m .+1<CR>==gi
-- inoremap <A-k> <Esc>:m .-2<CR>==gi
-- vnoremap <A-j> :m '>+1<CR>gv=gv
-- vnoremap <A-k> :m '<-2<CR>gv=gv
keymap("n", "<A-j>", ":m .+1<CR>==", opts)
keymap("n", "<A-k>", ":m .-2<CR>==", opts)
keymap("i", "<A-j>", "<Esc>:m .+1<CR>==gi", opts)
keymap("i", "<A-k>", "<Esc>:m .-2<CR>==gi", opts)
keymap("v", "<A-j>", ":m '>+1<CR>gv=gv", opts)
keymap("v", "<A-k>", ":m '<-2<CR>gv=gv", opts)
keymap("v", "p", '"_dP', opts)
keymap("x", "<A-j>", ":move '>+1<CR>gv-gv", opts)
keymap("x", "<A-k>", ":move '<-2<CR>gv-gv", opts)
-- keymap("x", "J", ":move '>+1<CR>gv-gv", opts)
-- keymap("x", "K", ":move '<-2<CR>gv-gv", opts)

-- Terminal --
-- Better terminal navigation
keymap("t", "<C-h>", "<C-\\><C-N><C-w>h", term_opts)
keymap("t", "<C-j>", "<C-\\><C-N><C-w>j", term_opts)
keymap("t", "<C-k>", "<C-\\><C-N><C-w>k", term_opts)
keymap("t", "<C-l>", "<C-\\><C-N><C-w>l", term_opts)

-- Nvim-Tree --
keymap("n", "<leader>e", ":NvimTreeToggle<CR>", opts)


-- LSP --
-- formating
keymap('n', '<leader>p', function() vim.lsp.buf.format({ async = true }) end, opts)
-- code action for auto-import
keymap('n', '<leader>.', vim.lsp.buf.code_action, opts)
-- open diagnostics float
keymap('n', '<leader>d', vim.diagnostic.open_float, opts)
-- yank diagnostics
keymap("n", "<leader>dy", function()
	local line = vim.api.nvim_win_get_cursor(0)[1] - 1 -- zero-indexed
	local diagnostics = vim.diagnostic.get(0, { lnum = line })

	if vim.tbl_isempty(diagnostics) then
		vim.notify(("No diagnostics on line %s"):format(line + 1), vim.log.levels.ERROR)
		return
	end

	local messages = {}
	for _, diag in ipairs(diagnostics) do
		table.insert(messages, diag.message)
	end

	if vim.fn.setreg("+", messages) ~= 0 then
		vim.notify(("An error occurred while copying diagnostics from line %s"):format(line + 1))
		return
	end

	vim.notify(([[Diagnostics from line %s copied to clipboard.

%s]]):format(line + 1, table.concat(messages, "\n")))
end, vim.tbl_extend("force", opts, {
	desc = "Copy current line diagnostics to system clipboard"
}))

-- Telescope --
-- telescope gd
local fzf = require("fzf-lua")
keymap("n", "gd", fzf.lsp_definitions, { desc = "fzf go-to-def" })
keymap("n", "gr", fzf.lsp_references, { desc = "fzf find-refs" })
keymap("n", "<leader>f", fzf.builtin, { desc = "fzf buildin" })
keymap("n", "<leader>ff", fzf.files, { desc = "fzf files" })
keymap("n", "<leader>fg", fzf.live_grep_glob, { desc = "fzf live-grep-glob" })
keymap("n", "<leader>fb", fzf.buffers, { desc = "fzf buffers" })
keymap("n", "<leader>fh", fzf.help_tags, { desc = "fzf help" })
