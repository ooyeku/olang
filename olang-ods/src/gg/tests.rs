//! The grammar end to end: specs in, scenes and SVG out.

use super::*;

fn frame() -> Data {
    let n = 60;
    let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let y: Vec<f64> = x.iter().map(|v| 2.0 * v + ((v * 7.0) % 5.0)).collect();
    let g: Vec<Option<String>> = (0..n).map(|i| Some(["a", "b", "c"][i % 3].to_string())).collect();
    Data::new(vec!["x".into(), "y".into(), "g".into()], vec![Column::Num(x), Column::Num(y), Column::Str(g)])
}

fn col(n: &str) -> Source {
    Source::Col(n.to_string())
}

fn layer(geom: &str) -> Layer {
    Layer { geom: geom.into(), inherit: true, legend: true, ..Default::default() }
}

fn spec(layers: Vec<Layer>) -> Spec {
    Spec {
        data: Some(frame()),
        mapping: vec![(Aes::X, col("x")), (Aes::Y, col("y")), (Aes::Color, col("g"))],
        layers,
        ..Default::default()
    }
}

#[test]
fn a_scatter_renders_points_a_legend_and_an_svg() {
    let s = render(&spec(vec![layer("point")]), &RenderOptions::default()).unwrap();
    let pts: usize = s.items.iter().map(|i| if let Item::Points(p) = i { p.xs.len() } else { 0 }).sum();
    // 60 data points plus three legend keys
    assert_eq!(pts, 63);
    let svg = s.to_svg();
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    assert!(svg.contains("<metadata id=\"olang-chart\">"));
    let json = s.to_json(&SceneJsonOptions::default());
    assert!(json.contains("\"type\":\"scene\"") && json.contains("\"op\":\"points\""));
}

#[test]
fn smooth_and_facets() {
    let mut sm = layer("smooth");
    sm.params.insert("method".into(), Param::Str("lm".into()));
    let mut sp = spec(vec![layer("point"), sm]);
    sp.facet = Some(FacetSpec { kind: "wrap".into(), rows: vec!["g".into()], ncol: Some(3), scales: "fixed".into(), ..Default::default() });
    let s = render(&sp, &RenderOptions::default()).unwrap();
    assert_eq!(s.panels.len(), 3);
    assert!(s.panels.windows(2).all(|w| w[0].y == w[1].y && w[0].x < w[1].x));
}

#[test]
fn unknown_column_says_which() {
    let mut sp = spec(vec![layer("point")]);
    sp.mapping[0] = (Aes::X, col("nope"));
    let e = render(&sp, &RenderOptions::default()).unwrap_err().to_string();
    assert!(e.contains("no such column") && e.contains("x, y, g"), "{e}");
}

#[test]
fn zoom_restricts_the_horizontal_axis() {
    let s = render(&spec(vec![layer("point")]), &RenderOptions { zoom: Some((10.0, 20.0, None)), ..Default::default() }).unwrap();
    let a = &s.panels[0].h_axis;
    assert!((a.lo - 10.0).abs() < 1e-9 && (a.hi - 20.0).abs() < 1e-9);
    // y fits what shows
    assert!(s.panels[0].v_axis.hi < 60.0);
}
