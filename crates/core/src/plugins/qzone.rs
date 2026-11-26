//! QQ Space host boundary: one service, existing plugin scopes and bounded image outputs.
use super::Services;
use anyhow::{ensure, Result};
use base64::Engine as _;
use std::path::{Component, Path, PathBuf};
use tokio::io::AsyncReadExt;
use yuantuan_plugin_api::*;

fn invalid() -> anyhow::Error {
    ToolError::new(
        ToolErrorKind::InvalidArguments,
        "图片须为云团图库或插件 artifacts 内的 PNG/JPEG/GIF/WebP，每张最多8MiB、合计24MiB",
    )
    .into()
}

pub(super) async fn execute(
    host: &Services,
    ctx: &ToolCtx,
    mut action: QzoneAction,
) -> Result<serde_json::Value> {
    ensure!(
        host.id == "qzone" || host.id == "qq",
        ToolError::new(ToolErrorKind::PermissionDenied, "此插件不能操作QQ空间")
    );
    ensure!(
        host.policy
            .allowed_chats
            .as_ref()
            .is_none_or(|a| a.contains(&ctx.chat_id)),
        ToolError::new(ToolErrorKind::PermissionDenied, "会话不在插件范围内")
    );
    let service = host
        .services
        .get::<dyn QzoneService>()
        .ok_or_else(|| ToolError::new(ToolErrorKind::Unavailable, "QQ空间适配器未启用"))?;
    if let QzoneAction::Publish { post, account_qq } = &mut action {
        post.validate()
            .map_err(|_| ToolError::new(ToolErrorKind::InvalidArguments, "说说参数不合规"))?;
        // Check the account owner before reading any files or transmitting a publish request.
        let status = service
            .execute(
                ctx,
                QzoneAction::CheckAccess {
                    account_qq: *account_qq,
                },
            )
            .await?;
        ensure!(
            status["qq"].as_u64() == Some(*account_qq),
            ToolError::new(
                ToolErrorKind::Unavailable,
                "QQ账号已改变，请重新确认发布账号"
            )
        );
        let root = host
            .directory
            .parent()
            .and_then(Path::parent)
            .ok_or_else(invalid)?;
        post.images = encode_images(root, &post.images).await?;
    }
    service.execute(ctx, action).await
}

pub(super) async fn encode_images(root: &Path, images: &[String]) -> Result<Vec<String>> {
    if images.is_empty() {
        return Ok(vec![]);
    }
    ensure!(images.len() <= 9, invalid());
    let root = tokio::fs::canonicalize(root).await.map_err(|_| invalid())?;
    let mut total = 0;
    let mut encoded = Vec::new();
    for original in images {
        ensure!(
            original.len() <= 2048 && !original.contains('\0') && !original.contains("://"),
            invalid()
        );
        let path = PathBuf::from(original);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        let file = tokio::fs::canonicalize(path).await.map_err(|_| invalid())?;
        let relative = file.strip_prefix(&root).map_err(|_| invalid())?;
        let parts: Vec<_> = relative.components().collect();
        ensure!(
            parts.iter().all(|c| matches!(c, Component::Normal(_))),
            invalid()
        );
        let permitted = (parts.len() >= 2 && parts[0].as_os_str() == "memes")
            || (parts.len() == 4
                && parts[0].as_os_str() == "plugins"
                && parts[2].as_os_str() == "artifacts");
        ensure!(
            permitted
                && tokio::fs::metadata(&file)
                    .await
                    .map_err(|_| invalid())?
                    .is_file(),
            invalid()
        );
        let mut bytes = Vec::new();
        tokio::fs::File::open(&file)
            .await
            .map_err(|_| invalid())?
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| invalid())?;
        total += bytes.len();
        ensure!(
            !bytes.is_empty() && bytes.len() <= 8 * 1024 * 1024 && total <= 24 * 1024 * 1024,
            invalid()
        );
        ensure!(
            matches!(
                image::guess_format(&bytes),
                Ok(image::ImageFormat::Png
                    | image::ImageFormat::Jpeg
                    | image::ImageFormat::Gif
                    | image::ImageFormat::WebP)
            ),
            invalid()
        );
        encoded.push(format!(
            "base64://{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ));
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn images_are_checked_and_encoded_for_a_separate_napcat_container() {
        let root = std::env::temp_dir().join(format!("qzone-images-{}", rand::random::<u64>()));
        tokio::fs::create_dir_all(root.join("plugins/nai-image/artifacts"))
            .await
            .unwrap();
        let image = root.join("plugins/nai-image/artifacts/out.png");
        image::RgbImage::new(2, 2).save(&image).unwrap();
        let encoded = encode_images(&root, &[image.to_string_lossy().into()])
            .await
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(encoded[0].strip_prefix("base64://").unwrap())
                .unwrap(),
            std::fs::read(&image).unwrap()
        );
        tokio::fs::write(root.join("secret.png"), b"private configuration")
            .await
            .unwrap();
        for path in [
            root.join("secret.png").to_string_lossy().into_owned(),
            "https://example.com/image.png".into(),
            "file:///etc/passwd".into(),
            "base64://AAAA".into(),
        ] {
            assert!(encode_images(&root, &[path]).await.is_err());
        }
        assert!(encode_images(&root, &["../outside.png".into()])
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
