// Fontshare hosts the original font; optional loading never delays the UI.
let displayFont: Promise<FontFace> | undefined
export function loadDisplayFont() {
  if (typeof FontFace === 'undefined' || displayFont) return
  const face = new FontFace('Cabinet Grotesk', 'url(https://cdn.fontshare.com/wf/J2PZYZURCR7HNQKXCZ4VXYA3K5FFCCLT/PZSZLWHMBCE7FFGOUYDEU33FAESUMA3X/JFDL5FBAQ2WMYL3LGKSCZKAIFCS2UQ63.woff2)', { weight: '100 900', style: 'normal', display: 'swap' })
  document.fonts.add(face)
  displayFont = face.load()
  void displayFont.catch(() => {
    // An offline console retains its local system font and all functionality.
    document.fonts.delete(face)
  })
}
