local ok, render_markdown = pcall(require, "render-markdown")
if not ok then
	return
end

render_markdown.setup({
	anti_conceal = {
		enabled = false,
	},
	pipe_table = {
		enabled = true,
		cell = "padded",
		style = "full",
	},
})
