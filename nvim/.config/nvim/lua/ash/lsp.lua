local lspconfig = require("lspconfig")

local on_attach = function(client, bufnr)
	-- format on save
	vim.api.nvim_create_autocmd("BufWritePre", {
		buffer = bufnr,
		callback = function()
			vim.lsp.buf.format({ async = false })
		end,
	})
end

-- rust-analyzer
lspconfig.rust_analyzer.setup({
	on_attach = on_attach,
	settings = {
		["rust-analyzer"] = {
			cargo = { allFeatures = true },
			checkOnSave = true,
			check = {
				command = "clippy",
			},
		},
	},
})

-- gopls
lspconfig.gopls.setup({
	on_attach = on_attach,
	settings = {
		gopls = {
			gofumpt = true,
			usePlaceholders = true,
			staticcheck = true,
		},
	},
})

-- React / TypeScript / JavaScript
lspconfig.ts_ls.setup({
	on_attach = on_attach,
	filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
	root_dir = lspconfig.util.root_pattern("package.json", "tsconfig.json", ".git"),
})

-- Eslint
lspconfig.eslint.setup({
	on_attach = on_attach,
	filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
	settings = {
		eslint = { workingDirectory = { mode = "auto" }, format = { enable = true } },
	},
	root_dir = require("lspconfig.util").root_pattern(
		".eslintrc.js", ".eslintrc.json", ".eslintignore", "package.json", ".git"
	),
})

-- lua
lspconfig.lua_ls.setup({
	on_attach = on_attach,
	root_dir = function(fname)
		return require("lspconfig.util").root_pattern("lua")(fname)
			or require("lspconfig.util").root_pattern(".git")(fname)
	end,
	settings = {
		Lua = {
			format = {
				enable = true,
			},
			diagnostics = {
				enable = true,
				globals = { "vim" },
			},
			workspace = {
				checkThirdParty = false,
			},
			telemetry = { enable = false },
		},
	},
})

vim.diagnostic.config({
	virtual_text = {
		prefix = '●', -- optional: customize symbol
		source = "if_many", -- show diagnostic source if multiple LSPs attached
		spacing = 2,
	},
	signs = true,
	underline = true,
	update_in_insert = false,
	severity_sort = true,
})
