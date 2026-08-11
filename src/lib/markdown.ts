/** Renders model output as HTML.
 *
 *  Everything here treats the source as untrusted. It is written by a model,
 *  which means it can be steered by anything the model has read — a web page, a
 *  file, a screenshot. Markdown that renders in this panel is therefore held to
 *  the same standard as markdown from a stranger.
 *
 *  The strategy is to make the renderer incapable of emitting anything
 *  dangerous, rather than to emit it and clean up afterwards. A parser that
 *  cannot produce a script tag needs no sanitizer pass to remove one, and cannot
 *  be defeated by a sanitizer bypass. This is markdown-it's own recommendation
 *  and its default.
 */
import MarkdownIt from "markdown-it";
import hljs from "highlight.js/lib/core";

// Registered individually rather than importing the "common" bundle, which carries
// thirty-odd grammars for a panel that answers questions about code someone is looking at.
// Add a language when a real answer needs it; an unregistered one falls through to plain
// text, which is a worse rendering rather than a broken one.
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import go from "highlight.js/lib/languages/go";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import swift from "highlight.js/lib/languages/swift";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

for (const [name, language] of [
  ["bash", bash],
  ["css", css],
  ["go", go],
  ["javascript", javascript],
  ["json", json],
  ["python", python],
  ["rust", rust],
  ["sql", sql],
  ["swift", swift],
  ["typescript", typescript],
  ["xml", xml],
  ["yaml", yaml],
] as const) {
  hljs.registerLanguage(name, language);
}

/**
 * Highlights a fenced block, or returns `null` to leave it plain.
 *
 * ## Why this is allowed to emit HTML when nothing else here is
 *
 * The rest of this module's security posture is that the renderer *cannot* produce HTML, so
 * there is nothing to sanitise. A highlighter breaks that by definition: it works by wrapping
 * tokens in `span`s. What replaces the guarantee is a narrower one — highlight.js escapes the
 * code it is given, so the only tags in its output are the ones it added.
 *
 * Verified rather than assumed, because its own security wiki warns about unescaped HTML.
 * Passing `<script>alert(1)</script>` through `hljs.highlight` returns
 * `&lt;script&gt;alert(1)&lt;/script&gt;` with the angle brackets escaped and no live tag:
 *
 *     hljs.highlight('<script>alert(1)</script>', { language: 'javascript' }).value
 *     // → '&lt;script&gt;<span class="hljs-title function_">alert</span>(…)&lt;/script&gt;'
 *
 * Re-run that check before changing versions. It is the whole reason this is safe.
 */
const highlight = (code: string, language: string): string | null => {
  if (!language || !hljs.getLanguage(language)) return null;

  try {
    // `ignoreIllegals` so a snippet that does not parse is still coloured as far as it got.
    // A model's code sample is frequently a fragment, and throwing here would drop the whole
    // block rather than the part that confused the grammar.
    return hljs.highlight(code, { language, ignoreIllegals: true }).value;
  } catch {
    return null;
  }
};

const renderer = new MarkdownIt({
  // The whole security posture in one line: raw HTML in the source is escaped
  // and shown as text, never parsed. Turning this on would require an external
  // sanitizer, and would make every future change a security review.
  html: false,
  // Bare URLs stay as plain text. They are still readable and copyable; they
  // just do not become anchors.
  linkify: false,
  // Models write single newlines and mean them. CommonMark would fold those into
  // one paragraph, which turns a short list of options into a wall of text.
  breaks: true,
  typographer: false,
  highlight: (code, language) => {
    const marked = highlight(code, language);
    if (marked === null) return "";

    // Returning the wrapper as well as the content, which is what markdown-it's own docs
    // prescribe: returning only the inner HTML makes it wrap the result again and escape it.
    // `hljs` is on the `pre` so the panel's stylesheet has one hook rather than one per
    // token class.
    return `<pre class="hljs"><code>${marked}</code></pre>`;
  },
});

// Images are disabled outright. `![](https://tracker/x.png)` in a reply would
// make the panel fetch a remote URL chosen by the model — a beacon that confirms
// you read the answer, and leaks your IP, from content we did not author. There
// is no use for images in a text panel that justifies keeping that open.
renderer.disable("image");

/** Per-render state for the link rules below.
 *
 *  A stack, not a single value, because `link_open` and `link_close` are separate
 *  calls and the destination is only known at open time while it is only needed
 *  at close time.
 *
 *  It lives in markdown-it's `env` rather than in a module-level variable. A
 *  module-level stack would be shared by every render in the process, so two
 *  renders interleaving — the streaming bubble and a finished turn re-rendering
 *  in the same tick — could pop each other's URLs and attach them to the wrong
 *  link. `env` is created per call, so the state cannot escape one render.
 */
type LinkEnv = { hrefs: string[] };

const hrefStack = (env: unknown): string[] | undefined =>
  (env as LinkEnv | undefined)?.hrefs;

/** A link renders as its text plus its destination, and is never clickable.
 *
 *  Two reasons, and either alone would be enough:
 *
 *  Navigation — this panel *is* the app's webview. Following a link inside it
 *  would replace Magi's own UI with a web page, with no back button and no way
 *  to return. Opening links properly means handing them to the OS browser, which
 *  is a separate piece of work with its own capability permission.
 *
 *  Honesty — markdown lets the visible text disagree with the destination, so
 *  `[docs.rust-lang.org](https://evil.example)` reads as trustworthy while
 *  pointing elsewhere. Showing the real URL next to the text makes that
 *  substitution visible instead of hidden.
 */
renderer.renderer.rules.link_open = (tokens, index, _options, env) => {
  // `attrGet` is typed `string | number | null` because markdown-it allows
  // numeric attribute values; an href is always a string in practice.
  hrefStack(env)?.push(String(tokens[index].attrGet("href") ?? ""));
  return '<span class="md-link">';
};

renderer.renderer.rules.link_close = (_tokens, _index, _options, env) => {
  const href = hrefStack(env)?.pop() ?? "";
  if (!href) return "</span>";
  return `</span><span class="md-url">${renderer.utils.escapeHtml(href)}</span>`;
};

/** Tables get a scrolling wrapper.
 *
 *  A table wide enough to overflow would otherwise stretch the whole thread and
 *  push every other message off screen, because the panel is a few hundred
 *  pixels wide and a table will not wrap. The wrapper confines the overflow to
 *  the table itself.
 */
renderer.renderer.rules.table_open = () => '<div class="md-table"><table>';
renderer.renderer.rules.table_close = () => "</table></div>";

/** Renders markdown to HTML that is safe to inject.
 *
 *  Safe because of the configuration above, not because of anything the caller
 *  does. Callers pass the result to `{@html}`; that is only sound as long as
 *  `html: false` stays set here.
 *
 *  The `env` is supplied here rather than left to default so the link rules
 *  always find their stack, and so each render gets its own.
 */
export const renderMarkdown = (source: string): string =>
  renderer.render(source, { hrefs: [] } satisfies LinkEnv);
