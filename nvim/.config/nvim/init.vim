set runtimepath^=~/.vim runtimepath+=~/.vim/after
let &packpath = &runtimepath
let g:python3_host_prog = '/usr/bin/python3'
let g:python2_host_prog = '/usr/bin/python'

syntax enable

lua require('init')
