local fn = vim.fn

-- Automatically install packer
local install_path = fn.stdpath "data" .. "/site/pack/packer/start/packer.nvim"
if fn.empty(fn.glob(install_path)) > 0 then
	PACKER_BOOTSTRAP = fn.system {
		"git",
		"clone",
		"--depth",
		"1",
		"https://github.com/wbthomason/packer.nvim",
		install_path,
	}
	print "Installing packer close and reopen Neovim..."
	vim.cmd [[packadd packer.nvim]]
end

-- Autocommand that reloads neovim whenever you save the plugins.lua file
vim.cmd [[
  augroup packer_user_config
    autocmd!
    autocmd BufWritePost plugins.lua source <afile> | PackerSync
  augroup end
]]

-- Use a protected call so we don't error out on first use
local status_ok, packer = pcall(require, "packer")
if not status_ok then
	return
end

-- Have packer use a popup window
packer.init {
	display = {
		open_fn = function()
			return require("packer.util").float { border = "rounded" }
		end,
	},
}

return packer.startup(function(use)
	-- utilities
	use "wbthomason/packer.nvim" -- Have packer manage itself
	use "nvim-lua/popup.nvim" -- An implementation of the Popup API from vim in Neovim
	use "nvim-lua/plenary.nvim" -- Useful lua functions used ny lots of plugins
	use "tpope/vim-sensible"
	use "tpope/vim-fugitive"  -- Git
	use 'nvim-tree/nvim-tree.lua'
	use 'ibhagwan/fzf-lua'

	-- UI
	use({ "ellisonleao/gruvbox.nvim" })
	use 'f-person/auto-dark-mode.nvim' -- switch light/dark to match system
	use 'rose-pine/neovim'

	-- cmp
	use "hrsh7th/nvim-cmp"      -- The completion plugin
	use "hrsh7th/cmp-buffer"    -- buffer completions
	use "hrsh7th/cmp-path"      -- path completions
	use "hrsh7th/cmp-cmdline"   -- cmdline completions
	use "saadparwaiz1/cmp_luasnip" -- snippet completions

	-- snippets
	use "L3MON4D3/LuaSnip"          --snippet engine
	use "rafamadriz/friendly-snippets" -- a bunch of snippets to use

	-- lsp
	use({
		"mason-org/mason.nvim",
		run = ":MasonUpdate",
		config = function()
			require("mason").setup()
		end,
	})
	use({
		"mason-org/mason-lspconfig.nvim",
		after = "mason.nvim",
		config = function()
			require("mason-lspconfig").setup({
				ensure_installed = { "lua_ls" },
				automatic_installation = false,
			})
		end,
	})
	use 'neovim/nvim-lspconfig'

	-- Automatically set up your configuration after cloning packer.nvim
	-- Put this at the end after all plugins
	if PACKER_BOOTSTRAP then
		require("packer").sync()
	end
end)
