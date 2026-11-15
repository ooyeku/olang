// compat: modules — share, use in every documented form, enum
// constructors through an import, re-exports, the namespace binding.
use lib.geometry { area, Shape, shape_area }
use lib.geometry { area as surface }
use lib.geometry {}
use lib.reexport { region }

println(area(3, 4))
println(surface(2, 5))
println(geometry.area(1, 1))
println(geometry.ORIGIN)
println(shape_area(Sq(3)) + shape_area(Rc(2, 3)))
println(region(6, 7))
type Pin = struct { at: Int }
impl Drawable for Pin { fn draw(self) = `pin@${self.at}` }
println(Pin { at: 4 }.draw())
println(is_err(attempt(() => geometry.scale(1))))
