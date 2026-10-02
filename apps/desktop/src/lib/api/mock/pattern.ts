// A camera stand-in for the browser mock: an animated SVG test pattern.

/** An animated test pattern standing in for a camera stream. */
export const TEST_PATTERN =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 320 180'>
<defs><linearGradient id='g' x1='0' x2='0' y1='0' y2='1'><stop offset='0' stop-color='#fff' stop-opacity='0'/><stop offset='1' stop-color='#fff' stop-opacity='.18'/></linearGradient></defs>
<rect width='320' height='180' fill='#0d0f14'/>
<g opacity='.8'>${["#d8d8d8", "#d8d000", "#00d0d0", "#00c000", "#d000d0", "#d00000", "#0000d0"]
    .map((c, i) => `<rect x='${i * 46}' width='46' height='118' fill='${c}'/>`)
    .join("")}</g>
<rect y='118' width='320' height='62' fill='#14161c'/>
<rect width='320' height='22' fill='url(#g)'><animate attributeName='y' from='-22' to='180' dur='2.4s' repeatCount='indefinite'/></rect>
<g fill='none' stroke='#4ade80' stroke-width='2'><rect x='196' y='38' width='54' height='54' rx='3'><animate attributeName='x' values='196;182;204;196' dur='5s' repeatCount='indefinite'/></rect></g>
<text x='16' y='154' fill='#9aa0ae' font-family='monospace' font-size='12'>apriltag · id 7 · 58 fps</text>
<circle cx='300' cy='150' r='5' fill='#fb7185'><animate attributeName='opacity' values='1;.2;1' dur='1.2s' repeatCount='indefinite'/></circle>
</svg>`);
