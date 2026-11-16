package assets

import "embed"

// Committed bundles keep Go builds independent of Node. Rebuild with make ui.
//
//go:embed page.js styles.css
var Files embed.FS
