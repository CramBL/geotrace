// Page and text setup shared by every display equation of the reference
// material: black glyphs on a transparent page. The window whitens the glyphs
// on upload and tints them to the theme's text colour when it draws them.

#let display-equation(body) = {
  set page(width: auto, height: auto, margin: 4pt, fill: none)
  set text(size: 11pt, fill: black)
  body
}
