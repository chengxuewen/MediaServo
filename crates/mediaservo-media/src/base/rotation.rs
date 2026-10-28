/// W3C-compatible video rotation angles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoRotation {
    #[default]
    Rotation0 = 0,
    Rotation90 = 90,
    Rotation180 = 180,
    Rotation270 = 270,
}
