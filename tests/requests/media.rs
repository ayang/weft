use axum_test::multipart::{MultipartForm, Part};
use loco_rs::testing::request::{request_with_config, RequestConfigBuilder};
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
