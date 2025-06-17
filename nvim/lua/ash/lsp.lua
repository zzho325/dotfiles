local lspconfig = require('lspconfig')

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
lspconfig.tsserver.setup({
	on_attach = on_attach,
	filetypes = { "javascript", "javascriptreact", "typescript", "typescriptreact" },
	root_dir = lspconfig.util.root_pattern("package.json", "tsconfig.json", ".git"),
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

