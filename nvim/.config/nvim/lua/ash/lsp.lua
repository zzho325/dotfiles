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

local cmp_nvim_lsp_ok, cmp_nvim_lsp = pcall(require, "cmp_nvim_lsp")
if not cmp_nvim_lsp_ok then
	return
end

local capabilities = vim.lsp.protocol.make_client_capabilities()
capabilities = cmp_nvim_lsp.default_capabilities(capabilities)

local lsp_defaults = {
	on_attach = on_attach,
	capabilities = capabilities,
}

lspconfig.util.default_config = vim.tbl_deep_extend(
	'force',
	lspconfig.util.default_config,
	lsp_defaults
)

-- rust-analyzer
lspconfig.rust_analyzer.setup({
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
	filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
	root_dir = lspconfig.util.root_pattern("package.json", "tsconfig.json", ".git"),
})

-- Eslint
lspconfig.eslint.setup({
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
