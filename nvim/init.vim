set runtimepath^=~/.vim runtimepath+=~/.vim/after
let &packpath = &runtimepath
let g:python3_host_prog = '/usr/bin/python3'
let g:python2_host_prog = '/usr/bin/python'

syntax enable

lua require('init')

" Plugins will be downloaded under the specified directory.
" call plug#begin(stdpath('data') . '/plugged')
" Declare the list of plugins.
" Plug 'tpope/vim-sensible'
" Plug 'junegunn/seoul256.vim'
" Plug 'nvim-lua/plenary.nvim'
" Plug 'nvim-telescope/telescope.nvim', { 'tag': '0.1.5' }
" Plug 'ellisonleao/gruvbox.nvim'
" Plug 'kdheepak/lazygit.nvim'
" Plug 'github/copilot.vim'
" Plug 'CopilotC-Nvim/CopilotChat.nvim'
" List ends here. Plugins become visible to Vim after this call.
" call plug#end()

