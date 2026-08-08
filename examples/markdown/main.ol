// Convert markdown to HTML. With a file argument, converts that file and
// prints the HTML; with none, converts a built-in sample that exercises
// every supported construct, then runs a self-check.
//
//   olang main.ol README.md > readme.html

use lib.blocks { to_html }

let args = unwrap(os.args())

if len(args) > 1 => {
    println(to_html(unwrap(fs.read_file(args[1]))))
}
else => {
    let sample = "# olang markdown\n" +
        "\n" +
        "A converter written *in* olang — headings, lists, `code`, and more.\n" +
        "\n" +
        "## What works\n" +
        "\n" +
        "- **Bold**, *italic*, and `inline code`\n" +
        "- Links like [the olang book](https://github.com/ooyeku/olang)\n" +
        "- Nested spans: **bold with *italic* inside**\n" +
        "\n" +
        "1. Ordered lists\n" +
        "2. With several items\n" +
        "\n" +
        "> Blockquotes join their lines\n" +
        "> into one paragraph.\n" +
        "\n" +
        "```\n" +
        "let x = 1 < 2 && 3 > 2   // code is escaped, not parsed\n" +
        "```\n" +
        "\n" +
        "---\n" +
        "\n" +
        "Escaping works in text too: AT&T, a < b, c > d.\n"

    println(to_html(sample))
}

// ── self-check: the converter's contract, verified on every run ──
test "markdown conversion" {
    assert_eq(to_html("# Title"), "<h1>Title</h1>")
    assert_eq(to_html("### Deep"), "<h3>Deep</h3>")
    assert_eq(to_html("plain text"), "<p>plain text</p>")
    assert_eq(to_html("**b** and *i*"), "<p><strong>b</strong> and <em>i</em></p>")
    assert_eq(to_html("`a < b`"), "<p><code>a &lt; b</code></p>")
    assert_eq(to_html("[x](http://y)"), "<p><a href=\"http://y\">x</a></p>")
    assert_eq(to_html("- one\n- two"), "<ul><li>one</li><li>two</li></ul>")
    assert_eq(to_html("1. a\n2. b"), "<ol><li>a</li><li>b</li></ol>")
    assert_eq(to_html("> q1\n> q2"), "<blockquote><p>q1 q2</p></blockquote>")
    assert_eq(to_html("---"), "<hr>")
    assert_eq(to_html("```\nx < y\n```"), "<pre><code>x &lt; y</code></pre>")
    assert_eq(to_html("a\nb\n\nc"), "<p>a b</p>\n<p>c</p>")
    assert_eq(to_html("**with *nested* span**"),
        "<p><strong>with <em>nested</em> span</strong></p>")
    assert_eq(to_html("AT&T x < y"), "<p>AT&amp;T x &lt; y</p>")
}
