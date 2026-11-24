// Bundle with esbuild and mathjax-full@3.2.2. Runs in macOS JavaScriptCore.
const {mathjax} = require('mathjax-full/js/mathjax.js');
const {TeX} = require('mathjax-full/js/input/tex.js');
const {SVG} = require('mathjax-full/js/output/svg.js');
const {liteAdaptor} = require('mathjax-full/js/adaptors/liteAdaptor.js');
const {RegisterHTMLHandler} = require('mathjax-full/js/handlers/html.js');
require('mathjax-full/js/input/tex/ams/AmsConfiguration.js');
require('mathjax-full/js/input/tex/newcommand/NewcommandConfiguration.js');
require('mathjax-full/js/input/tex/boldsymbol/BoldsymbolConfiguration.js');
require('mathjax-full/js/input/tex/cancel/CancelConfiguration.js');
require('mathjax-full/js/input/tex/color/ColorConfiguration.js');
require('mathjax-full/js/input/tex/mathtools/MathtoolsConfiguration.js');
require('mathjax-full/js/input/tex/textmacros/TextMacrosConfiguration.js');
require('mathjax-full/js/input/tex/unicode/UnicodeConfiguration.js');
const adaptor = liteAdaptor();
RegisterHTMLHandler(adaptor);
const input = new TeX({
    packages: ['base', 'ams', 'newcommand', 'boldsymbol', 'cancel', 'color', 'mathtools', 'textmacros', 'unicode'],
    maxBuffer: 8192,
    maxMacros: 1000,
    formatError: (_jax, error) => { throw error; },
});
const document = mathjax.document('', {InputJax: input, OutputJax: new SVG({fontCache:'none'})});
globalThis.openBearMath = (source, display) => {
    try {
        input.reset();
        const node = document.convert(source, {display, em: 15, ex: 7.5, containerWidth: 650});
        const svg = adaptor.firstChild(node);
        const [, top, width, height] = adaptor.getAttribute(svg, 'viewBox').split(/ +/).map(Number);
        const em = 0.015;
        const size = [width * em, height * em, -top * em];
        if (!size.every(Number.isFinite) || size[0] <= 0 || size[1] <= 0) return '';
        adaptor.setAttribute(svg, 'width', String(size[0]));
        adaptor.setAttribute(svg, 'height', String(size[1]));
        adaptor.setAttribute(svg, 'color', '#d6d7d8');
        return JSON.stringify([...size, adaptor.outerHTML(svg)]);
    } catch (_) {
        return '';
    }
};
