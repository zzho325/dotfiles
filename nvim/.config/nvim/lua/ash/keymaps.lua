local opts = { noremap = true, silent = true }
local term_opts = { silent = true }

-- Shorten function name
local keymap = vim.keymap.set

-- Abbreviations
vim.cmd("iabbrev fixm FIXME:")

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

-- Terminal --
-- Easier exiting
keymap("t", "<Esc>", [[<C-\><C-n>]])

-- Move text up and down
keymap("n", "<A-j>", ":m .+1<CR>==", opts)
keymap("n", "<A-k>", ":m .-2<CR>==", opts)
keymap("i", "<A-j>", "<Esc>:m .+1<CR>==gi", opts)
keymap("i", "<A-k>", "<Esc>:m .-2<CR>==gi", opts)
keymap("v", "<A-j>", ":m '>+1<CR>gv=gv", opts)
keymap("v", "<A-k>", ":m '<-2<CR>gv=gv", opts)
keymap("v", "p", '"_dP', opts)
keymap("x", "<A-j>", ":move '>+1<CR>gv-gv", opts)
keymap("x", "<A-k>", ":move '<-2<CR>gv-gv", opts)

-- Clipboard --
-- Yank to system clipboard
keymap("n", "<leader>y", '"+y', opts)
keymap("v", "<leader>y", '"+y', opts)
keymap( 'v', '<Leader>cp',
  [[:<C-u>let @+ = expand('%:.') . '(' . line("'<") . '-' . line("'>") . ')' <CR>:echo "Path with lines copied!"<CR>]],
  opts
)

-- Paste from system clipboard
keymap("n", "<leader>p", '"+p', opts)
keymap("n", "<leader>P", '"+P', opts)
keymap("v", "<leader>p", '"+p', opts)
keymap("v", "<leader>P", '"+P', opts)

-- Terminal --
-- Better terminal navigation
keymap("t", "<C-h>", "<C-\\><C-N><C-w>h", term_opts)
keymap("t", "<C-j>", "<C-\\><C-N><C-w>j", term_opts)
keymap("t", "<C-k>", "<C-\\><C-N><C-w>k", term_opts)
keymap("t", "<C-l>", "<C-\\><C-N><C-w>l", term_opts)

-- Tabs --
keymap("n", "<leader>tc", ":tabclose<CR>", opts)

-- Nvim-Tree --
keymap("n", "<leader>e", ":NvimTreeToggle<CR>", opts)
keymap("n", "<leader>ef", ":NvimTreeFindFile<CR>", opts)

-- LSP --
keymap('n', 'gi', vim.lsp.buf.implementation, { desc = 'go-to-implementation' })
keymap("i", "<C-h>", vim.lsp.buf.signature_help)
-- formating
keymap('n', '<leader>fm', function() vim.lsp.buf.format({ async = true }) end, opts)
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
local ok, fzf = pcall(require, "fzf-lua")
if ok then
	keymap("n", "gd", fzf.lsp_definitions, { desc = "fzf go-to-def" })
	keymap("n", "gr", fzf.lsp_references, { desc = "fzf find-refs" })
	keymap("n", "<leader>f", fzf.builtin, { desc = "fzf buildin" })
	keymap("n", "<leader>ff", fzf.files, { desc = "fzf files" })
	keymap("n", "<leader>fg", fzf.live_grep, { desc = "fzf live-grep-glob" })
	keymap("n", "<leader>fo", fzf.oldfiles, { desc = "fzf oldfiles" })
	keymap("n", "<leader>fb", fzf.buffers, { desc = "fzf buffers" })
	keymap("n", "<leader>fh", fzf.help_tags, { desc = "fzf help" })
	keymap("n", "<leader>gs", fzf.git_status, { desc = "fzf git status" })
	keymap("n", "<leader>gc", fzf.git_stash, { desc = "fzf git stash" })

	-- jj: open DiffviewOpen for a change using git's X^! (= X^..X) shorthand.
	local jj_dv_rev = nil
	local jj_dv_root = nil
	local function jj_root()
		local start = vim.fn.getcwd()
		local bufname = vim.api.nvim_buf_get_name(0)
		if bufname ~= "" and (vim.fn.filereadable(bufname) == 1 or vim.fn.isdirectory(bufname) == 1) then
			if vim.fn.isdirectory(bufname) == 1 then
				start = vim.fn.fnamemodify(bufname, ":p")
			else
				start = vim.fn.fnamemodify(bufname, ":p:h")
			end
		end
		local root = vim.fn.systemlist("cd " .. vim.fn.shellescape(start) .. " && jj root 2>/dev/null")[1]
		if not root or root == "" then
			vim.notify("jj: not in a repo", vim.log.levels.ERROR)
			return nil
		end
		return root
	end
	local function jj_cmd(root, cmd)
		return "cd " .. vim.fn.shellescape(root) .. " && " .. cmd
	end
	local function jj_diffview(rev, root)
		root = root or jj_root()
		if not root then return end
		local commit = vim.fn.systemlist(
			jj_cmd(root, "jj log -r '" .. rev .. "' --no-graph -T commit_id --limit 1 2>/dev/null")
		)[1]
		if commit and commit ~= "" then
			vim.cmd("lcd " .. vim.fn.fnameescape(root))
			vim.cmd("DiffviewOpen " .. commit .. "^!")
			jj_dv_rev = rev
			jj_dv_root = root
		else
			vim.notify("jj_diffview: could not resolve " .. rev, vim.log.levels.ERROR)
		end
	end

	-- Extract change ID (first field) from fzf selection.
	local function jj_change_id(selected)
		return selected[1]:match("^(%S+)")
	end

	-- jj stack: first field is the change id, used by preview/actions.
	local function jj_stack_cmd(root)
		return jj_cmd(root, "jj log -r 'main@origin..@' --no-graph"
			.. [[ -T 'change_id.shortest() ++ "  "]]
			.. [[ ++ if(local_bookmarks, local_bookmarks.map(|b| b.name()).join(" ") ++ " ", "")]]
			.. [[ ++ if(description, description.first_line(), "(no description " ++ change_id.shortest() ++ ")")]]
			.. [[ ++ "\n"']])
	end
	local jj_stack_fzf = {
		["--preview-window"] = "right,70%",
	}

	keymap("n", "<leader>jr", function()
		if not jj_dv_rev then return end
		local buf = vim.api.nvim_get_current_buf()
		local ft = vim.bo[buf].filetype
		if not ft:match("^Diffview") then return end
		vim.cmd("DiffviewClose")
		jj_diffview(jj_dv_rev, jj_dv_root)
	end, { desc = "refresh jj diffview" })


	keymap("n", "<leader>jb", function()
		local root = jj_root()
		if not root then return end
		fzf.fzf_exec(jj_stack_cmd(root), {
			prompt = "jj change> ",
			preview = jj_cmd(root, "jj diff -r {1} --git --color=always"),
			fzf_opts = jj_stack_fzf,
			actions = {
				["default"] = function(selected) jj_diffview(jj_change_id(selected), root) end,
			},
		})
	end, { desc = "jj stack diffview" })
	keymap("n", "<leader>jg", function()
		local root = jj_root()
		if not root then return end
		fzf.fzf_exec(jj_stack_cmd(root), {
			prompt = "jj review> ",
			preview = jj_cmd(root, "jj diff -r {1} --git --color=never"
				.. " | grep '^diff --git' | sed 's|.*b/||'"
				.. " | grep '\\.go$'"
				.. " | xargs -I@ dirname @"
				.. " | sort -u"
				.. " | sed 's|^|./|; s|$|/...|'"
				.. " | xargs goreview --diff main --changes-only --depth 4 --short 2>&1"),
			fzf_opts = jj_stack_fzf,
			actions = {
				["default"] = function(selected) jj_diffview(jj_change_id(selected), root) end,
			},
		})
	end, { desc = "jj stack goreview" })
end

-- Github --
function OpenGithubUrl()
	-- Get absolute file path
	local cur_file_path = vim.fn.expand('%:p')

	-- Get git repo root
	local git_root = vim.fn.systemlist('git rev-parse --show-toplevel')[1]
	if not git_root or git_root == '' then
		print("Not inside a Git repository")
		return
	end
	local rel_path = cur_file_path:sub(#git_root + 2)

	local origin_url = vim.fn.systemlist('git remote get-url origin')[1] or ''
	local user_repo =
		origin_url:match("github%.com[:/](.+)%.git") or
		origin_url:match("github%.com[:/](.+)")
	if not user_repo then
		vim.notify("Unable to determine GitHub repository from remote origin", vim.log.levels.ERROR)
		return
	end

	-- Get the default branch from local refs
	local head_ref = vim.fn.systemlist('git symbolic-ref refs/remotes/origin/HEAD')[1]
	local default_branch = head_ref and head_ref:match("refs/remotes/origin/(.+)") or 'master'

	local start_line = vim.fn.line('v')
	local end_line = vim.fn.line('.')
	if start_line > end_line then
		start_line, end_line = end_line, start_line
	end
	local linenum_str = string.format('#L%d-L%d', start_line, end_line)

	-- Final URL
	local url = string.format('%s/blob/%s/%s%s', 'https://github.com/' .. user_repo, default_branch, rel_path,
		linenum_str)
	-- Open URL
	vim.fn.system('open "' .. url .. '"')
end

function OpenCommitFromBlame()
	local line_number = vim.fn.line('.')
	local file_path = vim.fn.expand('%:p')

	local blame_output = vim.fn.system('git blame -L ' .. line_number .. ',' .. line_number .. ' ' .. file_path)
	local commit_hash = string.match(blame_output, '^[^%s]+')

	if not commit_hash or commit_hash == '' then
		vim.notify("Could not determine commit hash from git blame", vim.log.levels.ERROR)
		return
	end

	local origin_url = vim.fn.systemlist('git remote get-url origin')[1] or ''
	local user_repo =
		origin_url:match("github%.com[:/](.+)%.git") or
		origin_url:match("github%.com[:/](.+)")
	if not user_repo then
		vim.notify("Unable to determine GitHub repository from remote origin", vim.log.levels.ERROR)
		return
	end

	local url = 'https://github.com/' .. user_repo .. '/commit/' .. commit_hash
	vim.fn.system('open "' .. url .. '"')
end

keymap('n', '<leader>b', ':lua OpenGithubUrl()<CR>', opts)
keymap('n', '<leader>c', ':lua OpenCommitFromBlame()<CR>', opts)
