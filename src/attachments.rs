use std::{
    fs::{self, File},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
};

const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;

pub(crate) fn resolve(directory: &Path, url: &str) -> Option<PathBuf> {
    let name = url.strip_prefix("attachments/")?;
    let (stem, extension) = name.rsplit_once('.')?;
    if !stem.starts_with("image-")
        || !stem
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || !matches!(extension, "png" | "jpg" | "gif" | "webp")
    {
        return None;
    }
    let path = directory.join(name);
    path.symlink_metadata()
        .ok()?
        .file_type()
        .is_file()
        .then_some(path)
}

pub(crate) fn import(directory: &Path, source: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    File::open(source)?
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "L’immagine supera 12 MB",
        ));
    }
    let mut reader = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format()?;
    let extension = match reader.format() {
        Some(image::ImageFormat::Png) => "png",
        Some(image::ImageFormat::Jpeg) => "jpg",
        Some(image::ImageFormat::Gif) => "gif",
        Some(image::ImageFormat::WebP) => "webp",
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Scegli un’immagine PNG, JPEG, GIF o WebP",
            ));
        }
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(io::Error::other)?;
    fs::create_dir_all(directory)?;
    let mut file = tempfile::Builder::new()
        .prefix("image-")
        .suffix(&format!(".{extension}"))
        .tempfile_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    let path = file.into_temp_path().keep().map_err(|error| error.error)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("Nome dell’allegato non valido"))?;
    let alt = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("Immagine")
        .replace(['[', ']', '\n', '\r', '\\'], " ");
    Ok(format!("![{alt}](attachments/{name})"))
}

pub(crate) fn thumbnail(directory: &Path, text: &str) -> Option<PathBuf> {
    // Managed images use this prefix. Avoid parsing every text-only note in the list.
    if !text.contains("attachments/image-") {
        return None;
    }
    let root = markdown::to_mdast(text, &markdown::ParseOptions::gfm()).ok()?;
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        if let markdown::mdast::Node::Image(image) = node
            && let Some(path) = resolve(directory, &image.url)
        {
            return Some(path);
        }
        if let Some(children) = node.children() {
            pending.extend(children.iter().rev());
        }
    }
    None
}

pub(crate) fn export_references(
    directory: &Path,
    destination: &Path,
    markdown: &str,
) -> io::Result<()> {
    let root = markdown::to_mdast(markdown, &markdown::ParseOptions::gfm())
        .map_err(|error| io::Error::other(error.to_string()))?;
    let mut pending = vec![&root];
    while let Some(node) = pending.pop() {
        if let Some(children) = node.children() {
            pending.extend(children);
        }
        let markdown::mdast::Node::Image(image) = node else {
            continue;
        };
        let Some(source) = resolve(directory, &image.url) else {
            continue;
        };
        let folder = destination.with_file_name("attachments");
        let target = folder.join(
            source
                .file_name()
                .expect("managed image path has a filename"),
        );
        fs::create_dir_all(&folder)?;
        let mut pending = tempfile::NamedTempFile::new_in(&folder)?;
        io::copy(&mut File::open(&source)?, &mut pending)?;
        match pending.persist_noclobber(&target) {
            Ok(_) => {}
            Err(error)
                if error.error.kind() == io::ErrorKind::AlreadyExists
                    && target.symlink_metadata()?.file_type().is_file()
                    && fs::read(&target)? == fs::read(&source)? => {}
            Err(error) => return Err(error.error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_valid_images_and_resolves_only_managed_files() {
        let tmp = tempfile::tempdir().unwrap();
        let input = tmp.path().join("Foto [1].png");
        image::RgbImage::new(2, 2).save(&input).unwrap();
        let attachments = tmp.path().join("attachments");
        let markdown = import(&attachments, &input).unwrap();
        let url = markdown.split_once("](").unwrap().1.trim_end_matches(')');
        assert!(markdown.starts_with("![Foto  1 ]("));
        assert_eq!(
            fs::read(resolve(&attachments, url).unwrap()).unwrap(),
            fs::read(&input).unwrap()
        );
        assert!(resolve(&attachments, "attachments/../image-demo.png").is_none());
        assert!(resolve(&attachments, "https://example.com/image-demo.png").is_none());
        let exported = tmp.path().join("esportazione/nota.md");
        export_references(&attachments, &exported, &markdown).unwrap();
        let exported_image = resolve(&exported.with_file_name("attachments"), url).unwrap();
        assert_eq!(fs::read(exported_image).unwrap(), fs::read(&input).unwrap());
        export_references(&attachments, &exported, &markdown).unwrap();
        fs::write(&input, b"invalid").unwrap();
        assert!(import(&attachments, &input).is_err());
    }
}
