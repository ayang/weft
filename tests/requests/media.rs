use axum_test::multipart::{MultipartForm, Part};
use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
use serde_json::{json, Value};
use serial_test::serial;
use weft::{
    app::App,
    models::{
        posts::{self, PostParams},
        users,
    },
};

async fn login(request: &axum_test::TestServer, ctx: &loco_rs::app::AppContext) {
    users::Model::create_with_password(
        &ctx.db,
        &users::RegisterParams {
            email: "admin@example.com".to_string(),
            password: "change-me-12345".to_string(),
            name: "Admin".to_string(),
        },
    )
    .await
    .unwrap();
    let response = request
        .post("/admin/login")
        .form(&serde_json::json!({
            "email": "admin@example.com",
            "password": "change-me-12345"
        }))
        .await;
    assert_eq!(response.status_code(), 303);
}

async fn upload_test_media(
    request: &axum_test::TestServer,
    file_name: &str,
    bytes: Vec<u8>,
    mime: &str,
) -> Value {
    let form = MultipartForm::new().add_part(
        "file",
        Part::bytes(bytes).file_name(file_name).mime_type(mime),
    );
    let upload = request.post("/admin/media/upload").multipart(form).await;
    assert_eq!(upload.status_code(), 200, "upload body: {}", upload.text());
    serde_json::from_str(&upload.text()).unwrap()
}

#[tokio::test]
#[serial]
async fn media_upload_serves_images_and_video_markers() {
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        login(&request, &ctx).await;

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(b"weft-test-image");
        let form = MultipartForm::new().add_part(
            "file",
            Part::bytes(png)
                .file_name("cover.png")
                .mime_type("image/png"),
        );
        let upload = request.post("/admin/media/upload").multipart(form).await;
        assert_eq!(upload.status_code(), 200, "upload body: {}", upload.text());
        let payload: serde_json::Value = serde_json::from_str(&upload.text()).unwrap();
        let url = payload["url"].as_str().unwrap().to_string();
        let filename = payload["filename"].as_str().unwrap().to_string();
        assert_eq!(payload["kind"], "image");
        assert!(payload["markdown"]
            .as_str()
            .unwrap()
            .starts_with("![cover.png]("));

        let picker = request.get("/admin/api/media?q=cover").await;
        assert_eq!(picker.status_code(), 200);
        let picker_payload: serde_json::Value = serde_json::from_str(&picker.text()).unwrap();
        assert_eq!(picker_payload["items"][0]["kind"], "image");
        assert_eq!(picker_payload["items"][0]["url"], url);

        let file = request.get(&url).await;
        assert_eq!(file.status_code(), 200);
        assert_eq!(file.header("content-type"), "image/png");

        let mut jpeg = b"\xff\xd8\xff\xe0".to_vec();
        jpeg.extend_from_slice(b"actually-a-jpeg");
        let form = MultipartForm::new().add_part(
            "file",
            Part::bytes(jpeg)
                .file_name("actually-jpeg.png")
                .mime_type("image/png"),
        );
        let upload = request.post("/admin/media/upload").multipart(form).await;
        assert_eq!(upload.status_code(), 200, "upload body: {}", upload.text());
        let payload: serde_json::Value = serde_json::from_str(&upload.text()).unwrap();
        assert_eq!(payload["kind"], "image");
        assert_eq!(payload["mime"], "image/jpeg");
        let jpeg_filename = payload["filename"].as_str().unwrap().to_string();
        assert!(jpeg_filename.ends_with(".jpg"));
        let served = request.get(payload["url"].as_str().unwrap()).await;
        assert_eq!(served.header("content-type"), "image/jpeg");

        let mut mp4 = b"\x00\x00\x00\x18ftypmp42".to_vec();
        mp4.extend_from_slice(b"test-video");
        let form = MultipartForm::new().add_part(
            "file",
            Part::bytes(mp4)
                .file_name("clip.mp4")
                .mime_type("video/mp4"),
        );
        let upload = request.post("/admin/media/upload").multipart(form).await;
        assert_eq!(upload.status_code(), 200, "upload body: {}", upload.text());
        let payload: serde_json::Value = serde_json::from_str(&upload.text()).unwrap();
        assert_eq!(payload["kind"], "video");
        let marker = payload["video_marker"].as_str().unwrap().to_string();
        let video_filename = payload["filename"].as_str().unwrap().to_string();

        let post = posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "带视频的文章".to_string(),
                slug: "media-video".to_string(),
                summary: "媒体测试".to_string(),
                body_md: format!("# 视频\n\n{marker}\n"),
                status: posts::STATUS_PUBLISHED.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(post.status, posts::STATUS_PUBLISHED);
        let html = request.get("/posts/media-video").await.text();
        assert!(html.contains("<video"));
        assert!(html.contains("/media/"));
        assert!(!html.contains("[[video:"));

        let _ = std::fs::remove_file(weft::media::upload_dir().join(filename));
        let _ = std::fs::remove_file(weft::media::upload_dir().join(video_filename));
        let _ = std::fs::remove_file(weft::media::upload_dir().join(jpeg_filename));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn media_upload_accepts_attachments_and_renders_download_cards() {
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        login(&request, &ctx).await;

        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.extend_from_slice(b"weft-attachment-test");
        let form = MultipartForm::new().add_part(
            "file",
            Part::bytes(pdf)
                .file_name("团队报告.pdf")
                .mime_type("application/pdf"),
        );
        let upload = request.post("/admin/media/upload").multipart(form).await;
        assert_eq!(upload.status_code(), 200, "upload body: {}", upload.text());
        let payload: serde_json::Value = serde_json::from_str(&upload.text()).unwrap();
        assert_eq!(payload["kind"], "file");
        assert_eq!(payload["mime"], "application/pdf");
        assert_eq!(payload["type_label"], "PDF");
        let marker = payload["attachment_marker"].as_str().unwrap().to_string();
        assert!(marker.starts_with("[[file:"));
        assert!(marker.contains("团队报告.pdf"));
        let download_url = payload["download_url"].as_str().unwrap().to_string();
        assert!(download_url.ends_with("/download"));
        let filename = payload["filename"].as_str().unwrap().to_string();

        let download = request.get(&download_url).await;
        assert_eq!(download.status_code(), 200);
        assert_eq!(download.header("content-type"), "application/pdf");
        assert_eq!(download.header("x-content-type-options"), "nosniff");
        let disposition = download
            .header("content-disposition")
            .to_str()
            .unwrap()
            .to_string();
        assert!(disposition.starts_with("attachment;"));
        assert!(disposition.contains("filename*=UTF-8''"));
        assert!(disposition.contains("%E5%9B%A2"));

        let picker = request
            .get("/admin/api/media?q=%E6%8A%A5%E5%91%8A&kind=file")
            .await;
        assert_eq!(picker.status_code(), 200);
        let picker_payload: serde_json::Value = serde_json::from_str(&picker.text()).unwrap();
        assert_eq!(picker_payload["items"][0]["kind"], "file");
        assert_eq!(picker_payload["items"][0]["attachment_marker"], marker);

        let post = posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "带附件的文章".to_string(),
                slug: "media-attachment".to_string(),
                summary: "附件测试".to_string(),
                body_md: format!("# 附件\n\n{marker}\n"),
                status: posts::STATUS_PUBLISHED.to_string(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(post.status, posts::STATUS_PUBLISHED);
        let html = request.get("/posts/media-attachment").await.text();
        assert!(html.contains("class=\"attachment-card\""));
        assert!(html.contains("团队报告.pdf"));
        assert!(html.contains(&download_url));
        assert!(!html.contains("[[file:"));

        let unsafe_form = MultipartForm::new().add_part(
            "file",
            Part::bytes(b"<!doctype html><script>alert(1)</script>".to_vec())
                .file_name("evil.html")
                .mime_type("text/html"),
        );
        let rejected = request
            .post("/admin/media/upload")
            .multipart(unsafe_form)
            .await;
        assert_eq!(rejected.status_code(), 400);

        let _ = std::fs::remove_file(weft::media::upload_dir().join(filename));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn media_usage_marks_references_and_cleanup_only_deletes_unused_files() {
    let config = RequestConfigBuilder::new().save_cookies(true).build();
    request_with_config::<App, _, _>(config, |request, ctx| async move {
        login(&request, &ctx).await;

        let mut body_png = b"\x89PNG\r\n\x1a\n".to_vec();
        body_png.extend_from_slice(b"used-in-body");
        let body_media =
            upload_test_media(&request, "used-in-body.png", body_png, "image/png").await;

        let mut cover_png = b"\x89PNG\r\n\x1a\n".to_vec();
        cover_png.extend_from_slice(b"used-as-cover");
        let cover_media =
            upload_test_media(&request, "used-as-cover.png", cover_png, "image/png").await;

        let mut unused_pdf = b"%PDF-1.7\n".to_vec();
        unused_pdf.extend_from_slice(b"unused-attachment");
        let unused_media =
            upload_test_media(&request, "unused.pdf", unused_pdf, "application/pdf").await;

        let body_url = body_media["url"].as_str().unwrap().to_string();
        let cover_url = cover_media["url"].as_str().unwrap().to_string();
        let unused_url = unused_media["url"].as_str().unwrap().to_string();
        let body_filename = body_media["filename"].as_str().unwrap().to_string();
        let cover_filename = cover_media["filename"].as_str().unwrap().to_string();
        let unused_filename = unused_media["filename"].as_str().unwrap().to_string();

        posts::Model::create_post(
            &ctx.db,
            &PostParams {
                title: "引用媒体的文章".to_string(),
                slug: "media-usage-references".to_string(),
                summary: "媒体引用测试".to_string(),
                body_md: format!("# 正文\n\n![正文图]({body_url})\n"),
                status: posts::STATUS_PUBLISHED.to_string(),
                cover_image: cover_url.clone(),
                ..PostParams::default()
            },
        )
        .await
        .unwrap();

        let listing = request.get("/admin/api/media").await;
        assert_eq!(listing.status_code(), 200);
        let listing: Value = serde_json::from_str(&listing.text()).unwrap();
        let find = |filename: &str| {
            listing["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["filename"] == filename)
                .unwrap()
                .clone()
        };
        let body_item = find(&body_filename);
        let cover_item = find(&cover_filename);
        let unused_item = find(&unused_filename);
        assert_eq!(body_item["used"], true);
        assert_eq!(body_item["usage"][0]["kind"], "post");
        assert_eq!(body_item["usage"][0]["label"], "引用媒体的文章");
        assert_eq!(cover_item["used"], true);
        assert!(cover_item["usage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|usage| usage["kind"] == "cover"));
        assert_eq!(unused_item["used"], false);
        assert!(unused_item["usage"].as_array().unwrap().is_empty());

        // Deleting a referenced file is refused, and the bytes stay available.
        let denied = request
            .post("/admin/api/media/delete")
            .json(&json!({ "ids": [body_item["id"]] }))
            .await;
        assert_eq!(denied.status_code(), 200, "denied: {}", denied.text());
        let denied: Value = serde_json::from_str(&denied.text()).unwrap();
        assert_eq!(denied["deleted"], 0);
        assert_eq!(denied["skipped"][0]["id"], body_item["id"]);
        assert_eq!(request.get(&body_url).await.status_code(), 200);

        // One unused file can be removed by id.
        let deleted = request
            .post("/admin/api/media/delete")
            .json(&json!({ "ids": [unused_item["id"]] }))
            .await;
        assert_eq!(deleted.status_code(), 200, "delete: {}", deleted.text());
        let deleted: Value = serde_json::from_str(&deleted.text()).unwrap();
        assert_eq!(deleted["deleted"], 1);
        assert_eq!(request.get(&unused_url).await.status_code(), 404);

        // Upload another orphan, then remove every unused file in one call.
        let mut extra_png = b"\x89PNG\r\n\x1a\n".to_vec();
        extra_png.extend_from_slice(b"extra-unused");
        let extra_media =
            upload_test_media(&request, "extra-unused.png", extra_png, "image/png").await;
        let extra_filename = extra_media["filename"].as_str().unwrap().to_string();
        let cleanup = request
            .post("/admin/api/media/delete")
            .json(&json!({ "unused": true }))
            .await;
        assert_eq!(cleanup.status_code(), 200, "cleanup: {}", cleanup.text());
        let cleanup: Value = serde_json::from_str(&cleanup.text()).unwrap();
        assert_eq!(cleanup["deleted"], 1);
        assert_eq!(
            request
                .get(extra_media["url"].as_str().unwrap())
                .await
                .status_code(),
            404
        );
        assert_eq!(request.get(&body_url).await.status_code(), 200);
        assert_eq!(request.get(&cover_url).await.status_code(), 200);

        let _ = std::fs::remove_file(weft::media::upload_dir().join(&body_filename));
        let _ = std::fs::remove_file(weft::media::upload_dir().join(&cover_filename));
        let _ = std::fs::remove_file(weft::media::upload_dir().join(&extra_filename));
    })
    .await;
}
