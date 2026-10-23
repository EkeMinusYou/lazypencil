use lazypencil::geometry::{Affine, Rect};

#[test]
fn affine_composition_and_rect_bounds_keep_document_coordinates() {
    let matrix = Affine::translation(100.0, 200.0)
        .then(Affine::rotation(90.0))
        .then(Affine::scale(2.0, 3.0));
    let point = matrix.point(10.0, 20.0);
    assert!((point.0 - 160.0).abs() < 0.001);
    assert!((point.1 - 180.0).abs() < 0.001);
    let bounds = Rect::new(0.0, 0.0, 10.0, 20.0).transformed(matrix);
    assert!((bounds.width - 60.0).abs() < 0.001);
    assert!((bounds.height - 20.0).abs() < 0.001);
    assert_eq!(
        Rect::new(0.0, 0.0, 10.0, 20.0).intersection(Rect::new(5.0, 5.0, 20.0, 20.0)),
        Rect::new(5.0, 5.0, 5.0, 15.0)
    );
}
