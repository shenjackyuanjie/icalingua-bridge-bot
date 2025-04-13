/// 音乐平台的定义
///
/// 谁知道还有啥呢
///
/// + non_exhaustive
///
/// ```typescript
/// export type MusicType = "qq" | "163" | "migu" | "kugou" | "kuwo";
/// ```
#[non_exhaustive]
pub enum MusicPlatform {
    /// QQ音乐
    QQ,
    /// 网易云音乐
    Netease,
    /// 酷狗音乐
    Kugou,
    /// 酷我音乐
    Kuwo,
    /// 咪咕音乐
    Migu,
}

impl MusicPlatform {
    pub fn name(&self) -> &str {
        match self {
            MusicPlatform::QQ => "qq",
            MusicPlatform::Netease => "163",
            MusicPlatform::Kugou => "kugou",
            MusicPlatform::Kuwo => "kuwo",
            MusicPlatform::Migu => "migu",
        }
    }
}
