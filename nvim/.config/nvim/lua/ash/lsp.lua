-- lua/user/lsp.lua
local M = {}

function M.setup()
	local util_ok, util = pcall(require, "lspconfig.util")
	if not util_ok then
		util = require("lspconfig.util") -- let the error be visible if lspconfig isn't available
	end

	-- common on_attach: format on save
	local on_attach = function(client, bufnr)
		if client.supports_method and client.supports_method("textDocument/formatting") then
			vim.api.nvim_create_autocmd("BufWritePre", {
				buffer = bufnr,
				callback = function()
					-- prefer the new formatting API
					vim.lsp.buf.format({ bufnr = bufnr, async = false })
				end,
			})
		end
	end

	-- capabilities (for nvim-cmp)
	local capabilities = vim.lsp.protocol.make_client_capabilities()
	local ok_cmp, cmp_nvim_lsp = pcall(require, "cmp_nvim_lsp")
	if ok_cmp and cmp_nvim_lsp and cmp_nvim_lsp.default_capabilities then
		capabilities = cmp_nvim_lsp.default_capabilities(capabilities)
	end

	-- require mason & mason-lspconfig
	local mason_ok, mason = pcall(require, "mason")
	if not mason_ok then
		vim.notify("mason not installed — LSP setup aborted", vim.log.levels.WARN)
		return
	end
	local mlsp_ok, mason_lspconfig = pcall(require, "mason-lspconfig")
	if not mlsp_ok then
		vim.notify("mason-lspconfig not installed — LSP setup aborted", vim.log.levels.WARN)
		return
	end
	local lspconfig_ok, lspconfig = pcall(require, "lspconfig")
	if not lspconfig_ok then
		vim.notify("nvim-lspconfig not installed — LSP setup aborted", vim.log.levels.WARN)
		return
	end

	-- ensure these servers are installed via mason
	mason.setup()
	mason_lspconfig.setup({
		ensure_installed = { "rust_analyzer", "gopls", "tsserver", "eslint", "lua_ls" },
		automatic_installation = false, -- we will handle setup via handlers
	})

	-- default handler: attach basic on_attach & capabilities
	mason_lspconfig.setup_handlers({
		-- default handler
		function(server_name)
			lspconfig[server_name].setup({
				on_attach = on_attach,
				capabilities = capabilities,
			})
		end,

		-- rust_analyzer
		["rust_analyzer"] = function()
			lspconfig.rust_analyzer.setup({
				on_attach = on_attach,
				capabilities = capabilities,
				cmd = { "rust-analyzer" },
				filetypes = { "rust" },
				root_dir = util.root_pattern("Cargo.toml", ".git"),
				settings = {
					["rust-analyzer"] = {
						cargo = { allFeatures = true },
						checkOnSave = { command = "clippy" },
					},
				},
			})
		end,

		-- gopls
		["gopls"] = function()
			lspconfig.gopls.setup({
				on_attach = on_attach,
				capabilities = capabilities,
				cmd = { "gopls" },
				filetypes = { "go", "gomod", "gowork", "gotmpl" },
				root_dir = util.root_pattern("go.work", "go.mod", ".git"),
				settings = {
					gopls = {
						gofumpt = true,
						usePlaceholders = true,
						staticcheck = true,
					},
				},
			})
		end,

		-- tsserver
		["tsserver"] = function()
			lspconfig.tsserver.setup({
				on_attach = on_attach,
				capabilities = capabilities,
				cmd = { "typescript-language-server", "--stdio" },
				filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
				root_dir = util.root_pattern("package.json", "tsconfig.json", ".git"),
			})
		end,

		-- eslint
		["eslint"] = function()
			lspconfig.eslint.setup({
				on_attach = on_attach,
				capabilities = capabilities,
				cmd = { "vscode-eslint-language-server", "--stdio" },
				filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
				root_dir = util.root_pattern(".eslintrc.js", ".eslintrc.json", "package.json", ".git"),
				settings = {
					eslint = {
						workingDirectory = { mode = "auto" },
						format = { enable = false },
					},
				},
			})
		end,

		-- lua_ls
		["lua_ls"] = function()
			lspconfig.lua_ls.setup({
				on_attach = on_attach,
				capabilities = capabilities,
				cmd = { "lua-language-server" },
				filetypes = { "lua" },
				root_dir = util.root_pattern(".git", ".config", "lua"),
				settings = {
					Lua = {
						format = { enable = true },
						diagnostics = {
							enable = true,
							globals = { "vim" },
						},
						workspace = { checkThirdParty = false },
						telemetry = { enable = false },
					},
				},
			})
		end,
	})

	-- FileType-specific options (preserve your tab/shiftwidth rules)
	vim.api.nvim_create_autocmd("FileType", {
		pattern = { "javascript", "typescript", "javascriptreact", "typescriptreact" },
		callback = function()
			vim.bo.shiftwidth = 2
			vim.bo.tabstop = 2
			vim.bo.expandtab = true
		end,
	})

	-- Diagnostic display
	vim.diagnostic.config({
		virtual_text = {
			prefix = "●",
			source = "if_many",
			spacing = 2,
		},
		signs = true,
		underline = true,
		update_in_insert = false,
		severity_sort = true,
	})

	-- Optional: small LspAttach logger while you verify things
	vim.api.nvim_create_autocmd("LspAttach", {
		callback = function(args)
			local client = vim.lsp.get_client_by_id(args.data.client_id)
			vim.notify(("LspAttach: %s (id=%d) root=%s"):format(
				client.name, client.id, (client.config and client.config.root_dir) or "<nil>"
			), vim.log.levels.DEBUG)
		end,
	})
end

return M
