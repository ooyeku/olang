// a module: what it shares, and a private helper
share fn area(w, h) = scale(w * h)
share let ORIGIN = (0, 0)
share type Shape = enum { Sq(Int), Rc(Int, Int) }
share trait Drawable { fn draw(self) -> String }
share fn shape_area(s) = match s { Sq(n) => n * n, Rc(w, h) => w * h }
fn scale(v) = v
