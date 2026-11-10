use std::sync::Arc;

use super::*;
use crate::geom::{Cxform, Mat2d};
use crate::image::ImageData;
use crate::tess::Mesh;

const STAGE: [f32; 4] = [320.0, 0.0, 2240.0, 1080.0];

fn list(commands: Vec<Command>) -> RenderList {
    RenderList { commands, viewport: (2560, 1080), background: None, stage: Some(STAGE) }
}

fn mesh(vertices: Vec<[f32; 2]>, indices: Vec<u32>) -> Arc<Mesh> {
    Arc::new(Mesh { vertices, indices, paint: Vec::new() })
}

fn rect(r: [f32; 4]) -> Arc<Mesh> {
    mesh(vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]], vec![0, 1, 2, 0, 2, 3])
}

fn draw(mesh: Arc<Mesh>, paint: Paint) -> Command {
    Command::Draw(DrawItem {
        mesh,
        paint,
        transform: Mat2d::IDENTITY,
        cxform: Cxform::IDENTITY,
        blend: BlendMode::Normal,
    })
}

fn opaque_black() -> Paint {
    Paint::Solid([0.0, 0.0, 0.0, 1.0])
}

#[test]
fn only_an_opaque_rectangle_over_the_whole_stage_covers_it() {
    let lib = crate::display::tests::empty_lib();
    let mut b = OpaqueBitmaps::default();
    let full = rect(STAGE);
    assert!(list(vec![draw(full.clone(), opaque_black())]).covers_stage(&lib, &mut b));
    assert!(list(vec![draw(rect([320.4, 0.3, 2239.6, 1079.8]), opaque_black())]).covers_stage(&lib, &mut b));
    assert!(!list(vec![draw(rect([330.0, 0.0, 2240.0, 1080.0]), opaque_black())]).covers_stage(&lib, &mut b));
    assert!(!list(vec![draw(full.clone(), Paint::Solid([0.0, 0.0, 0.0, 0.99]))]).covers_stage(&lib, &mut b));
    let mut faded = draw(full.clone(), opaque_black());
    if let Command::Draw(d) = &mut faded {
        d.cxform = Cxform { mult: [1.0, 1.0, 1.0, 0.5], add: [0.0; 4] };
    }
    assert!(!list(vec![faded]).covers_stage(&lib, &mut b));
    let mut added = draw(full.clone(), opaque_black());
    if let Command::Draw(d) = &mut added {
        d.blend = BlendMode::Add;
    }
    assert!(!list(vec![added]).covers_stage(&lib, &mut b));
    let masked = vec![
        Command::MaskBegin,
        draw(rect([0.0, 0.0, 10.0, 10.0]), opaque_black()),
        Command::MaskEnd,
        draw(full.clone(), opaque_black()),
    ];
    assert!(!list(masked).covers_stage(&lib, &mut b));
    let after_mask = vec![
        Command::MaskBegin,
        draw(rect([0.0, 0.0, 10.0, 10.0]), opaque_black()),
        Command::MaskEnd,
        Command::MaskPopBegin,
        draw(rect([0.0, 0.0, 10.0, 10.0]), opaque_black()),
        Command::MaskPopEnd,
        draw(full.clone(), opaque_black()),
    ];
    assert!(list(after_mask).covers_stage(&lib, &mut b));
    let l = mesh(
        vec![[320.0, 0.0], [2240.0, 0.0], [2240.0, 540.0], [1280.0, 540.0], [1280.0, 1080.0], [320.0, 1080.0]],
        vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 5],
    );
    assert!(!list(vec![draw(l, opaque_black())]).covers_stage(&lib, &mut b));
    let (cx, cy, r) = (1280.0, 540.0, 2000.0);
    let turned = mesh(vec![[cx - r, cy], [cx, cy - r], [cx + r, cy], [cx, cy + r]], vec![0, 1, 2, 0, 2, 3]);
    assert!(!list(vec![draw(turned, opaque_black())]).covers_stage(&lib, &mut b));
    let mut no_stage = list(vec![draw(full, opaque_black())]);
    no_stage.stage = None;
    assert!(!no_stage.covers_stage(&lib, &mut b));
    assert!(!list(vec![draw(mesh(vec![[0.0, 0.0]], vec![0, 1, 2]), opaque_black())]).covers_stage(&lib, &mut b));
}

#[test]
fn a_bitmap_covers_only_when_every_texel_is_opaque() {
    let mut lib = crate::display::tests::empty_lib();
    let mut rgba = vec![255u8; 4 * 4 * 4];
    lib.add_image_movie("opaque.png", ImageData { width: 4, height: 4, rgba: rgba.clone(), source: "o".into() })
        .unwrap();
    let opaque = lib.images().len() - 1;
    rgba[4 * 5 + 3] = 254;
    lib.add_image_movie("hole.png", ImageData { width: 4, height: 4, rgba, source: "h".into() }).unwrap();
    let hole = lib.images().len() - 1;
    let bitmap = |image: ImageId, r: [u32; 4]| Paint::Bitmap {
        bitmap: BitmapDef { image, rect: r, size: [r[2] - r[0], r[3] - r[1]] },
        matrix: Mat2d::IDENTITY,
        repeat: false,
        smooth: true,
    };
    let mut b = OpaqueBitmaps::default();
    let covers = |p: Paint, b: &mut OpaqueBitmaps| list(vec![draw(rect(STAGE), p)]).covers_stage(&lib, b);
    assert!(covers(bitmap(opaque, [0, 0, 4, 4]), &mut b));
    assert!(!covers(bitmap(hole, [0, 0, 4, 4]), &mut b));
    assert!(covers(bitmap(hole, [2, 0, 4, 4]), &mut b));
    assert!(!covers(bitmap(hole + 1, [0, 0, 4, 4]), &mut b));
    assert!(!covers(bitmap(opaque, [4, 0, 8, 4]), &mut b));
}

fn first_bounds(l: &RenderList) -> [f32; 4] {
    let Command::Draw(d) = &l.commands[0] else { panic!("not a draw") };
    rectangle(d).expect("a rectangle")
}

#[test]
fn backdrops_over_the_whole_stage_reach_the_window_edges() {
    let dimmer = rect([-10.0, -10.0, 1290.0, 730.0]);
    let at_stage = Mat2d { a: 1.5, b: 0.0, c: 0.0, d: 1.5, tx: 320.0, ty: 0.0 };
    let place = |c: Command| match c {
        Command::Draw(mut d) => {
            d.transform = at_stage;
            Command::Draw(d)
        }
        other => other,
    };
    let mut l = list(vec![place(draw(dimmer.clone(), Paint::Solid([0.0, 0.0, 0.0, 0.6])))]);
    assert!(l.shows_beside_stage());
    l.extend_backdrops();
    let b = first_bounds(&l);
    assert!(b.iter().zip([0.0, -15.0, 2560.0, 1095.0]).all(|(x, y)| (x - y).abs() < 1e-3), "{b:?}");
    let mut l = RenderList {
        commands: vec![place(draw(dimmer.clone(), Paint::Solid([0.0, 0.0, 0.0, 0.6])))],
        viewport: (1920, 1080),
        background: None,
        stage: Some([0.0, 0.0, 1920.0, 1080.0]),
    };
    assert!(!l.shows_beside_stage());
    let before = first_bounds(&l);
    l.extend_backdrops();
    assert_eq!(first_bounds(&l), before);
    let bitmap = Paint::Bitmap {
        bitmap: BitmapDef { image: 0, rect: [0, 0, 4, 4], size: [4, 4] },
        matrix: Mat2d::IDENTITY,
        repeat: false,
        smooth: true,
    };
    for commands in [
        vec![draw(rect(STAGE), bitmap)],
        vec![draw(rect([330.0, 0.0, 2240.0, 1080.0]), opaque_black())],
        vec![
            Command::MaskBegin,
            draw(rect(STAGE), opaque_black()),
            Command::MaskEnd,
            draw(rect(STAGE), opaque_black()),
        ],
    ] {
        let mut l = list(commands);
        let before = l.commands.clone();
        l.extend_backdrops();
        for (a, b) in l.commands.iter().zip(&before) {
            if let (Command::Draw(a), Command::Draw(b)) = (a, b) {
                assert!(Arc::ptr_eq(&a.mesh, &b.mesh));
            }
        }
    }
    let gradient = Paint::Gradient(Arc::new(crate::shape::GradientPaint {
        kind: crate::shape::GradientKind::Linear,
        matrix: Mat2d::IDENTITY,
        spread: 0,
        interpolation: 0,
        stops: vec![(0.0, [0.0, 0.0, 0.0, 0.5]), (1.0, [0.0, 0.0, 0.0, 0.8])],
    }));
    let mut sliced = (*rect(STAGE)).clone();
    sliced.paint = sliced.vertices.clone();
    let mut l = list(vec![draw(Arc::new(sliced), gradient.clone())]);
    l.extend_backdrops();
    assert_eq!(first_bounds(&l), STAGE);
    let mut l = list(vec![draw(rect(STAGE), gradient)]);
    l.extend_backdrops();
    assert_eq!(first_bounds(&l), [0.0, 0.0, 2560.0, 1080.0]);
}
