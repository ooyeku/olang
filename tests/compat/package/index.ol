//! The root module of the compatibility fixture package. Every shared
//! item here is `@stable`: its name and signature are promised for 1.x.

/// Area of a w by h rectangle.
/// @stable
share fn area(w, h) = w * h

/// A greeting, with a default and a named argument.
/// @stable
share fn greet(name, greeting = "Hello") = `${greeting}, ${name}`

/// A point on the plane.
/// @stable
share type Point = struct { x: Int, y: Int }

/// A shape.
/// @stable
share type Shape = enum { Sq(Int), Rc(Int, Int) }

/// Something with a size.
/// @stable
share trait Sized { fn size(self) -> Int }

/// The origin.
/// @stable
share let ORIGIN = (0, 0)

/// Re-exported from lib.shapes under another name.
share use lib.shapes { perimeter as edge }

/// May still change: not held by --against.
/// @experimental
share fn draft(x) = x
