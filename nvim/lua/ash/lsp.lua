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

