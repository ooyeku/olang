// compat: the text modules — str, re, base64, json, toml, csv, md.

let s = "  The quick brown fox  "
println(str.trim(s))
println(str.split("a,b,,c", ","))
println(str.join(["x", "y", "z"], "-"))
println(str.replace("aaa", "a", "b"))
println(str.contains("haystack", "st"))
println(str.starts_with("olang", "ol") && str.ends_with("olang", "ng"))
println(str.index_of("hello", "l"))
println(str.substring("hello", 1, 3))
println(str.pad_start("7", 3, "0"))
println(str.repeat("ab", 3))
println(str.chars("héllo"))
println(str.length("héllo"))
println(str.lines("one\ntwo\n"))
println(str.words("  two   words "))
println(str.parse_int("42"))
println(is_err(str.parse_int("4x2")))
println(str.capitalize("olang"))
println(str.reverse("abc"))

println(re.is_match("[0-9]+", "abc123"))
println(re.find_all("[0-9]+", "a1b22c333"))
println(re.replace("[aeiou]", "banana", "_"))
println(re.split("\\s*,\\s*", "a , b,c"))
println(re.captures("(\\w+)@(\\w+)", "me@host"))

println(base64.encode("hello, olang"))
println(base64.decode("aGVsbG8sIG9sYW5n"))

let doc = json.parse("{\"name\": \"olang\", \"tags\": [\"a\", \"b\"], \"n\": 1.5, \"none\": null}")
println(doc)
println(json.stringify(#{ "b": [1, 2], "a": true }))
println(json.prettify("{\"a\":1}"))
println(is_err(json.parse("{nope")))

println(toml.parse("title = \"x\"\n[server]\nport = 8080\n"))
println(toml.stringify(#{ "name": "pkg", "version": "1.0.0" }))

let table = csv.parse_with_headers("name,qty\napple,3\npear,5\n")
println(table)
println(csv.stringify([["a", "b"], ["1", "2,3"]]))

// a report may gain fields in a minor release: pin the fields, not the record
println(md.render("# Title\n\nSome *emphasis*.\n").html)
println(md.slug("Hello, World!"))
println(md.headings("# One\n## Two\n") |> map((h) => `${h.level} ${h.text} ${h.slug}`))
