//! The grammar end to end: specs in, scenes and SVG out.

use super::*;

fn frame() -> Data {
    let n = 60;
    let x: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let y: Vec<f64> = x.iter().map(|v| 2.0 * v + ((v * 7.0) % 5.0)).collect();
    let g: Vec<Option<String>> = (0..n)
        .map(|i| Some(["a", "b", "c"][i % 3].to_string()))
        .collect();
    Data::new(
        vec!["x".into(), "y".into(), "g".into()],
        vec![Column::Num(x), Column::Num(y), Column::Str(g)],
    )
}

fn col(n: &str) -> Source {
    Source::Col(n.to_string())
}

fn layer(geom: &str) -> Layer {
    Layer {
        geom: geom.into(),
        inherit: true,
        legend: true,
        ..Default::default()
    }
}

fn spec(layers: Vec<Layer>) -> Spec {
    Spec {
        data: Some(frame()),
        mapping: vec![
            (Aes::X, col("x")),
            (Aes::Y, col("y")),
            (Aes::Color, col("g")),
        ],
        layers,
        ..Default::default()
    }
}

#[test]
fn a_scatter_renders_points_a_legend_and_an_svg() {
    let s = render(&spec(vec![layer("point")]), &RenderOptions::default()).unwrap();
    let pts: usize = s
        .items
        .iter()
        .map(|i| {
            if let Item::Points(p) = i {
                p.xs.len()
            } else {
                0
            }
        })
        .sum();
    // 60 data points plus three legend keys
    assert_eq!(pts, 63);
    let svg = s.to_svg();
    assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    assert!(svg.contains("<metadata class=\"olang-chart\">"));
    let json = s.to_json(&SceneJsonOptions::default());
    assert!(json.contains("\"type\":\"scene\"") && json.contains("\"op\":\"points\""));
}

#[test]
fn smooth_and_facets() {
    let mut sm = layer("smooth");
    sm.params.insert("method".into(), Param::Str("lm".into()));
    let mut sp = spec(vec![layer("point"), sm]);
    sp.facet = Some(FacetSpec {
        kind: "wrap".into(),
        rows: vec!["g".into()],
        ncol: Some(3),
        scales: "fixed".into(),
        ..Default::default()
    });
    let s = render(&sp, &RenderOptions::default()).unwrap();
    assert_eq!(s.panels.len(), 3);
    assert!(
        s.panels
            .windows(2)
            .all(|w| w[0].y == w[1].y && w[0].x < w[1].x)
    );
}

#[test]
fn unknown_column_says_which() {
    let mut sp = spec(vec![layer("point")]);
    sp.mapping[0] = (Aes::X, col("nope"));
    let e = render(&sp, &RenderOptions::default())
        .unwrap_err()
        .to_string();
    assert!(e.contains("no such column") && e.contains("x, y, g"), "{e}");
}

#[test]
fn zoom_restricts_the_horizontal_axis() {
    let s = render(
        &spec(vec![layer("point")]),
        &RenderOptions {
            zoom: Some((10.0, 20.0, None)),
            ..Default::default()
        },
    )
    .unwrap();
    let a = &s.panels[0].h_axis;
    assert!((a.lo - 10.0).abs() < 1e-9 && (a.hi - 20.0).abs() < 1e-9);
    // y fits what shows
    assert!(s.panels[0].v_axis.hi < 60.0);
}

#[test]
fn free_scales_bin_each_panel_over_its_own_range() {
    // two groups six orders of magnitude apart: each panel's bins its own
    let n = 40;
    let x: Vec<f64> = (0..n)
        .map(|i| {
            if i < 20 {
                i as f64 / 20.0
            } else {
                1000.0 + i as f64 * 50.0
            }
        })
        .collect();
    let g: Vec<Option<String>> = (0..n)
        .map(|i| Some(if i < 20 { "small" } else { "big" }.to_string()))
        .collect();
    let mut h = layer("histogram");
    h.params.insert("bins".into(), Param::Num(10.0));
    let mut sp = Spec {
        data: Some(Data::new(
            vec!["x".into(), "g".into()],
            vec![Column::Num(x), Column::Str(g)],
        )),
        mapping: vec![(Aes::X, col("x"))],
        layers: vec![h],
        ..Default::default()
    };
    sp.facet = Some(FacetSpec {
        kind: "wrap".into(),
        rows: vec!["g".into()],
        scales: "free".into(),
        ..Default::default()
    });
    let b = build::build(&sp, &RenderOptions::default()).unwrap();
    let f = &b.layers[0].frame;
    let (xmin, xmax) = (f.num(Aes::Xmin).unwrap(), f.num(Aes::Xmax).unwrap());
    // panels by label: "big" first, "small" second
    let small: Vec<usize> = (0..f.n).filter(|&i| f.panel[i] == 1).collect();
    assert!(small.len() >= 10, "{} bins", small.len());
    assert!(
        small.iter().all(|&i| xmin[i] > -0.2 && xmax[i] < 1.2),
        "the small panel's bins span its own range"
    );
    assert_eq!(
        small
            .iter()
            .map(|&i| f.num(Aes::Y).unwrap()[i])
            .sum::<f64>(),
        20.0
    );
    // fixed scales: one range for both (the small panel all in one bin)
    sp.facet.as_mut().unwrap().scales = "fixed".into();
    let b = build::build(&sp, &RenderOptions::default()).unwrap();
    let f = &b.layers[0].frame;
    let ys: Vec<f64> = (0..f.n)
        .filter(|&i| f.panel[i] == 1)
        .map(|i| f.num(Aes::Y).unwrap()[i])
        .filter(|y| *y > 0.0)
        .collect();
    assert_eq!(ys, vec![20.0]);
}

#[test]
fn a_constant_mapping_groups_one_line_across_categories_and_keys_a_legend() {
    let x: Vec<Option<String>> = ["a", "b", "c", "d"]
        .iter()
        .map(|s| Some(s.to_string()))
        .collect();
    let data = Data::new(
        vec!["x".into(), "y".into()],
        vec![Column::Str(x), Column::Num(vec![1.0, 3.0, 2.0, 4.0])],
    );
    let mut line = layer("line");
    line.mapping = vec![(
        Aes::Group,
        Source::Const("group".into(), Column::Num(vec![1.0])),
    )];
    let mut pts = layer("point");
    pts.mapping = vec![(
        Aes::Color,
        Source::Const("color".into(), Column::Str(vec![Some("all".into())])),
    )];
    let sp = Spec {
        data: Some(data),
        mapping: vec![(Aes::X, col("x")), (Aes::Y, col("y"))],
        layers: vec![line, pts],
        ..Default::default()
    };
    let b = build::build(&sp, &RenderOptions::default()).unwrap();
    assert!(
        b.layers[0].frame.group.iter().all(|g| *g == 0),
        "one group: {:?}",
        b.layers[0].frame.group
    );
    let s = render(&sp, &RenderOptions::default()).unwrap();
    let paths = s
        .items
        .iter()
        .filter(|i| matches!(i, Item::Path { pts, .. } if pts.len() == 4))
        .count();
    assert_eq!(paths, 1, "the line joins the four points");
    assert!(
        s.to_svg().contains(">all</text>"),
        "the constant keys a legend"
    );
    // without it a categorical x splits the line into one point a group: nothing drawn
    let mut sp2 = sp.clone();
    sp2.layers[0].mapping.clear();
    let s2 = render(&sp2, &RenderOptions::default()).unwrap();
    assert_eq!(
        s2.items
            .iter()
            .filter(|i| matches!(i, Item::Path { pts, .. } if pts.len() == 4))
            .count(),
        0
    );
}

#[test]
fn a_linewidth_mapping_widens_lines_and_its_scale_sets_the_range() {
    let mut sp = spec(vec![layer("line")]);
    sp.mapping.push((Aes::Linewidth, col("g")));
    sp.scales.push(ScaleSpec {
        aes: "linewidth".into(),
        kind: "linewidth".into(),
        params: [("range".to_string(), Param::Nums(vec![0.5, 2.0]))]
            .into_iter()
            .collect(),
    });
    let b = build::build(&sp, &RenderOptions::default()).unwrap();
    let lws = b.layers[0].vis.lws.clone().expect("mapped");
    let mut seen: Vec<f64> = lws.clone();
    seen.sort_by(|a, b| a.partial_cmp(b).unwrap());
    seen.dedup();
    assert_eq!(seen, vec![0.5 * 2.8, 1.25 * 2.8, 2.0 * 2.8]);
    let s = render(&sp, &RenderOptions::default()).unwrap();
    let widths: Vec<f64> = s
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Path {
                sw, tips: Some(_), ..
            } = i
            {
                Some(*sw)
            } else {
                None
            }
        })
        .collect();
    assert!(
        widths.contains(&(2.0 * 2.8)) && widths.contains(&(0.5 * 2.8)),
        "{widths:?}"
    );
}

#[test]
fn an_abline_is_clipped_to_the_panel() {
    use super::geom::clip_line;
    // y = 3x − 20 inside x ∈ [10, 30], y ∈ [10, 40]: from x = 10 to x = 20
    assert_eq!(
        clip_line(3.0, -20.0, (10.0, 30.0), (10.0, 40.0)),
        Some((10.0, 20.0))
    );
    // flat lines inside and outside
    assert_eq!(
        clip_line(0.0, 15.0, (0.0, 1.0), (10.0, 40.0)),
        Some((0.0, 1.0))
    );
    assert_eq!(clip_line(0.0, 50.0, (0.0, 1.0), (10.0, 40.0)), None);
    // missing the box altogether
    assert_eq!(clip_line(1.0, 100.0, (0.0, 1.0), (0.0, 1.0)), None);
    // a falling line
    let (a, b) = clip_line(-2.0, 10.0, (0.0, 10.0), (0.0, 10.0)).unwrap();
    assert!((a - 0.0).abs() < 1e-12 && (b - 5.0).abs() < 1e-12);
}

#[test]
fn text_on_a_tile_takes_the_ink_that_reads() {
    use super::geom::ink_on;
    use super::scene::{Rgba, Role};
    let surface = Appearance::Dark.role(Role::Surface);
    let dark = Appearance::Light.role(Role::Ink);
    let light = Appearance::Dark.role(Role::Ink);
    assert_eq!(ink_on(Rgba::hex("#f6f1e7").unwrap(), surface), dark);
    assert_eq!(ink_on(Rgba::hex("#c0412b").unwrap(), surface), light);
    assert_eq!(ink_on(Rgba::hex("#1d4f9e").unwrap(), surface), light);
    // a translucent fill reads as what shows through it
    assert_eq!(
        ink_on(Rgba::hex("#ffffff").unwrap().with_alpha(0.05), surface),
        light
    );
}

#[test]
fn a_scenes_json_is_compact_and_says_the_same() {
    let s = render(
        &spec(vec![layer("point"), layer("line")]),
        &RenderOptions::default(),
    )
    .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&s.to_json(&SceneJsonOptions::default())).unwrap();
    assert_eq!(json["v"], serde_json::json!(2));
    let ops = json["ops"].as_array().unwrap();
    // a path's vertices as steps in tenths of a pixel
    let (path_item, path_op) = (
        s.items
            .iter()
            .find_map(|i| {
                if let Item::Path {
                    pts, tips: Some(_), ..
                } = i
                {
                    Some(pts.clone())
                } else {
                    None
                }
            })
            .unwrap(),
        ops.iter().find(|o| o["op"] == "path").unwrap(),
    );
    let d: Vec<i64> = path_op["d"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    let (mut x, mut y) = (0i64, 0i64);
    for (k, p) in path_item.iter().enumerate() {
        x += d[2 * k];
        y += d[2 * k + 1];
        assert!((x as f64 / 10.0 - p.0).abs() <= 0.05 && (y as f64 / 10.0 - p.1).abs() <= 0.05);
    }
    // tooltip fields: the group's one value as a String, few values as levels and indices
    let line_tips = path_op["tips"].as_array().unwrap();
    assert!(line_tips.iter().any(|c| c.is_string()), "{line_tips:?}");
    let pts = ops.iter().find(|o| o["op"] == "points").unwrap();
    let g = pts["tips"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c.is_object())
        .expect("g as levels");
    assert_eq!(g["lv"].as_array().unwrap().len(), 3);
}

#[test]
fn a_moved_marks_tooltip_says_its_data_not_where_it_was_moved() {
    // jittered across a categorical x, stacked bars: the values said are the data's
    let mut sp = spec(vec![layer("jitter")]);
    sp.mapping = vec![(Aes::X, col("g")), (Aes::Y, col("y"))];
    let s = render(&sp, &RenderOptions::default()).unwrap();
    let tips = s
        .items
        .iter()
        .find_map(|i| {
            if let Item::Points(p) = i {
                p.tips.clone()
            } else {
                None
            }
        })
        .unwrap();
    assert!(
        tips.cols[0]
            .iter()
            .all(|v| ["a", "b", "c"].contains(&v.as_str())),
        "{:?}",
        &tips.cols[0][..5]
    );
    let mut c = spec(vec![layer("col")]);
    c.mapping = vec![
        (Aes::X, col("g")),
        (Aes::Y, col("y")),
        (Aes::Fill, col("g")),
    ];
    let b = build::build(&c, &RenderOptions::default()).unwrap();
    let l = &b.layers[0];
    let src = l
        .fields
        .iter()
        .find(|(n, _)| n == "y")
        .map(|f| f.1.clone())
        .unwrap();
    let said: Vec<String> = (0..3).map(|i| build::tip_value(&b, l, &src, i)).collect();
    assert_eq!(said, vec!["0", "4", "8"]);
}
