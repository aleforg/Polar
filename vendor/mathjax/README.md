# MathJax SVG renderer

The embedded bundle uses MathJax 3.2.2 under the Apache 2.0 license. Polar evaluates it locally in JavaScriptCore and uses AppKit to draw the resulting SVG.

To regenerate it, copy `entry.cjs` to a scratch directory, install `mathjax-full@3.2.2` and `esbuild@0.25.12` with npm in that directory, and run:

```sh
npx esbuild entry.cjs --bundle --minify --legal-comments=inline --platform=browser --outfile=svg.js
```

Copy the generated `svg.js` back into this directory.
