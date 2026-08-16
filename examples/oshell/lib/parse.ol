// oshell parser and expansion.
//
// Tokens become a list of chain links:
//   { pipe: [stage, ...], op }
// where each stage is { argv } (already-expanded strings), and op is the
// connective to the NEXT link: ";", "&&", or "||". Redirections are
// collected per pipeline: { in_file, out_file, append }.
//
// Expansion order per word token, matching the useful part of the shell
// rules: $? and $VAR/${VAR} first, then a leading ~, then globbing if the
// word contains * or ? (a pattern with no matches stays literal, like
// bash without nullglob). Raw tokens (single-quoted) skip all three.

share fn parse_chains(tokens, last_code) = {
    let mut chains = []
    let mut stage = []          // token list for the stage being built
    let mut stages = []         // completed stages of the current pipeline
    let mut in_file = ""
    let mut out_file = ""
    let mut append = false
    let mut redirect = ""       // pending redirect operator awaiting its target
    let mut err = ""

    for tok in tokens {
        if err == "" => {
            if redirect != "" && tok.q != "o" => {
                let target = expand_one(tok, last_code)
                if redirect == "<" => { in_file = target }
                else => {
                    out_file = target
                    append = redirect == ">>"
                }
                redirect = ""
            } else => if tok.q == "o" => {
                if redirect != "" => {
                    err = "oshell: syntax error: redirect needs a file name"
                } else => if tok.t == "<" || tok.t == ">" || tok.t == ">>" => {
                    redirect = tok.t
                } else => if tok.t == "|" => {
                    if len(stage) == 0 => { err = "oshell: syntax error near |" }
                    else => {
                        stages = stages + [stage]
                        stage = []
                    }
                } else => {
                    // ; && || — close the pipeline into a chain link
                    if len(stage) == 0 && len(stages) == 0 => {
                        if tok.t != ";" => { err = "oshell: syntax error near " + tok.t }
                    } else => if len(stage) == 0 => {
                        err = "oshell: syntax error near " + tok.t
                    } else => {
                        stages = stages + [stage]
                        chains = chains + [{
                            pipe: stages, op: tok.t,
                            in_file: in_file, out_file: out_file, append: append
                        }]
                        stage = []
                        stages = []
                        in_file = ""
                        out_file = ""
                        append = false
                    }
                }
            } else => {
                stage = stage + [tok]
            }
        }
    }

    if err == "" && redirect != "" => { err = "oshell: syntax error: redirect needs a file name" }
    if err == "" && len(stages) > 0 && len(stage) == 0 => { err = "oshell: syntax error near |" }
    if err != "" => Err(err)
    else => {
        if len(stage) > 0 => {
            stages = stages + [stage]
        }
        if len(stages) > 0 => {
            chains = chains + [{
                pipe: stages, op: ";",
                in_file: in_file, out_file: out_file, append: append
            }]
        }
        Ok(chains)
    }
}

// Expand one token to a list of argv words (globs can splice several).
share fn expand_token(tok, last_code) = {
    if tok.q == "r" => [tok.t]
    else => {
        let word = expand_one(tok, last_code)
        if str.contains(word, "*") || str.contains(word, "?") => {
            match fs.glob(word) {
                Ok(matches) => if len(matches) > 0 => sort(matches) else => [word],
                Err(e) => [word]
            }
        } else => [word]
    }
}

// Variable and tilde expansion for a single token (no globbing).
share fn expand_one(tok, last_code) = {
    if tok.q == "r" => tok.t
    else => {
        let expanded = expand_vars(tok.t, last_code)
        if str.starts_with(expanded, "~") => {
            let home = match os.home_dir() { Ok(h) => h, Err(e) => "" }
            if home == "" => expanded
            else => if expanded == "~" => home
            else => if str.starts_with(expanded, "~/") => home + str.substring(expanded, 1, str.length(expanded))
            else => expanded
        } else => expanded
    }
}

fn expand_vars(text, last_code) = {
    let cs = str.chars(text)
    let n = len(cs)
    let mut out = ""
    let mut i = 0
    while i < n {
        if cs[i] == "$" && i + 1 < n => {
            if cs[i + 1] == "?" => {
                out = out + to_string(last_code)
                i = i + 2
            } else => if cs[i + 1] == "{" => {
                let mut name = ""
                let mut j = i + 2
                while j < n && cs[j] != "}" {
                    name = name + cs[j]
                    j = j + 1
                }
                if j < n => {
                    out = out + env_or_empty(name)
                    i = j + 1
                } else => {
                    out = out + cs[i]
                    i = i + 1
                }
            } else => if is_name_char(cs[i + 1]) => {
                let mut name = ""
                let mut j = i + 1
                while j < n && is_name_char(cs[j]) {
                    name = name + cs[j]
                    j = j + 1
                }
                out = out + env_or_empty(name)
                i = j
            } else => {
                out = out + cs[i]
                i = i + 1
            }
        } else => {
            out = out + cs[i]
            i = i + 1
        }
    }
    out
}

fn env_or_empty(name) = match os.get_env(name) {
    Ok(v) => v,
    Err(e) => ""
}

fn is_name_char(c) = str.contains("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_", c)
