// primes-worker.ol — an olang program that runs INSIDE a Web Worker.
// There is no DOM here: the whole surface is dom.on_message (requests
// in) and dom.post (values out). Heavy compute lives on this thread;
// the page's frame loop never stalls.

fn is_prime(n) = {
    if n < 2 => false
    else => {
        let mut d = 2
        let mut prime = true
        while d * d <= n {
            if n % d == 0 => {
                prime = false
                d = n
            }
            d = d + 1
        }
        prime
    }
}

dom.on_message((m) => {
    let upto = map_get(m, "upto")
    let chunk = upto / 20
    let mut count = 0
    let mut lo = 2
    while lo < upto {
        let hi = if lo + chunk > upto => upto else => lo + chunk
        let mut n = lo
        while n < hi {
            if is_prime(n) => {
                count = count + 1
            }
            n = n + 1
        }
        lo = hi
        // Posted mid-compute: the page sees the bar move while this
        // thread keeps grinding.
        dom.post(#{ "kind": "progress", "done": lo, "upto": upto, "count": count })
    }
    dom.post(#{ "kind": "done", "upto": upto, "count": count })
})

dom.post(#{ "kind": "ready" })
