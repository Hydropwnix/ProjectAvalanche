use super::*;

#[test]
fn the_own_ghost_fades_out_on_the_player() {
    let mut f = HoloFade::default();
    for _ in 0..6 {
        f.update(HOLOGRAM_RACER_TYPE, 1.0, 100.0, 4, |_| 0.0);
    }
    assert_eq!(f.fade, 1.0);
    let far = f.update(HOLOGRAM_RACER_TYPE, 0.0, 100.0, 4, |max| max);
    assert_eq!((f.alpha, far.x, far.y), (1.0, 0.0, 0.0));
    let on = f.update(HOLOGRAM_RACER_TYPE, 0.0, 0.5, 4, |max| max);
    assert_eq!((f.alpha, on.x), (0.0, 1.0));
    assert!((on.y - 0.7).abs() < 1e-6);
    f.update(HOLOGRAM_RACER_TYPE, 0.0, 1.05, 4, |_| 0.0);
    assert!((f.alpha - 0.5).abs() < 1e-6, "{}", f.alpha);
    let mid = f.update(HOLOGRAM_RACER_TYPE, 0.0, 10.0, 4, |_| 0.0);
    assert!((mid.x - 0.5).abs() < 1e-6, "{}", mid.x);
    f.update(2, 0.0, 0.5, 4, |_| 0.0);
    assert_eq!(f.alpha, 1.0, "an explore ghost is not faded on the player");
}
