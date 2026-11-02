-- Observer's bridge. Observer installs it before every start, under a name drawn for that start,
-- and overwrites any change. No name or description metadata: scripts read those first.
fx_version 'cerulean'
game 'common'

server_scripts {
    'server.lua',
    'heap.js',
}
