-- Observer's bridge. Observer installs it before every start and overwrites any change.
fx_version 'cerulean'
game 'common'

name 'observer'
description 'Relays Observer lifecycle events as server events and reports the server health to Observer'

server_scripts {
    'server.lua',
    'heap.js',
}
