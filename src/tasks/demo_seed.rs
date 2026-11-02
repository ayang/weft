use loco_rs::prelude::*;
use serde_json::json;

use crate::{
    charts,
    models::{chart_specs, embeddings, posts},
    util,
};

pub struct DemoSeed;

#[async_trait]
impl Task for DemoSeed {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "demo:seed".to_string(),
            detail: "Create three published demo posts, tags, chunks and a static ECharts chart. Idempotent.".to_string(),
        }
    }

    async fn run(&self, ctx: &AppContext, _vars: &task::Vars) -> Result<()> {
        let demo_posts = vec![
            (
                "hello-weft",
                "在 Weft 上，人与 AI 一起写作",
                "这是一个纯动态、单一 Rust 二进制的个人站点：文章、图表、llms.txt 和 Agent API 都由 Loco 在请求时生成。",
                r#"# 为什么要做 Weft

传统博客把「写给人看」和「写给机器读」当成两件事。Weft 想把它们放回同一条生产线：人负责判断与表达，AI 负责提炼、作图和索引，站点同时服务人类读者与 Agent。

## 三条设计原则

1. **动态优先**：没有静态构建产物，所有输出按请求生成。
2. **证据优先**：关键结论明确标注，图表保留数据更新时间。
3. **小机器优先**：单二进制 + SQLite WAL，后台任务限流。

## 接下来

下一篇文章会介绍图表配方和可刷新数据源。

```rust
let site = Weft::new("sqlite").await?;
```
"#,
                vec!["Rust".to_string(), "AI".to_string()],
            ),
            (
                "chart-recipes",
                "图表不是图片，而是可刷新的配方",
                "chart_specs 保存意图与 ECharts 配置，chart_data_caches 保存带 ETag 的最新数据；定时任务只刷新数据，不重新构建页面。",
                r#"# 图表配方

图表由三部分组成：

- **intent**：这张图回答什么问题。
- **spec**：ECharts option（受控 JSON）。
- **data**：来自 static、只读 SQL 或白名单 HTTP 的最新数据。

## 为什么要缓存数据

小 VPS 不应该在每次页面访问时打外部 API。`chart_data_caches` 保存最近一次成功结果和 ETag；过期后仍可服务旧数据，同时由后台任务刷新。

## 失败会怎样

刷新失败会记录 `error_message`、`failure_count` 和 `next_attempt_at`，下一次调度按指数退避重试。

> 图表容器只拿到 spec + data，不执行任何模型生成的代码。
"#,
                vec!["图表".to_string(), "Rust".to_string(), "AI".to_string()],
            ),
            (
                "agent-readable",
                "让 Agent 读懂你的站点",
                "llms.txt、/posts/:slug.md 与 /api/agent/* 都是动态端点，支持 ETag；站内问答返回引用，方便人类与 Agent 交叉验证。",
                r#"# 面向 Agent 的阅读接口

人类读者看到的是 HTML；Agent 可以直接读取 Markdown 或结构化 JSON。

## 端点

- `/llms.txt`：内容地图与更新策略。
- `/posts/:slug.md`：单篇文章 Markdown。
- `/api/agent/posts`：按更新时间增量拉取。

## 边界

Agent API 默认只读；生产环境应设置 `AGENT_API_TOKEN` 并按分钟限流。AI 只生成受控 JSON/文本，不执行代码。
"#,
                vec!["AI".to_string(), "Agent".to_string()],
            ),
        ];

        for (slug, title, summary, body, tags) in demo_posts {
            if posts::Model::find_by_slug(&ctx.db, slug).await.is_ok() {
                continue;
            }
            let params = posts::PostParams {
                title: title.to_string(),
                slug: slug.to_string(),
                summary: summary.to_string(),
                body_md: body.to_string(),
                status: posts::STATUS_PUBLISHED.to_string(),
                pinned: slug == "hello-weft",
                cover_image: String::new(),
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
            };
            let post = posts::Model::create_post(&ctx.db, &params).await?;
            let chunks = util::chunk_text(&post.body_md, 900, 120);
            let inputs = chunks
                .into_iter()
                .enumerate()
                .map(|(index, content)| embeddings::ChunkInput {
                    index: i64::try_from(index).unwrap_or(0),
                    content,
                    embedding: None,
                })
                .collect::<Vec<_>>();
            embeddings::Model::replace_for_post(&ctx.db, post.id, "keyword-fallback", &inputs)
                .await?;
        }

        if chart_specs::Model::for_post(
            &ctx.db,
            posts::Model::find_by_slug(&ctx.db, "chart-recipes")
                .await?
                .id,
        )
        .await?
        .is_empty()
        {
            let post = posts::Model::find_by_slug(&ctx.db, "chart-recipes").await?;
            let params = chart_specs::ChartParams {
                title: "每月发布节奏（示例）".to_string(),
                intent: "展示示例数据；真实使用时改成只读 SQL 或白名单 HTTP。".to_string(),
                kind: "bar".to_string(),
                data_source_type: "static".to_string(),
                data_source_config: json!({
                    "data": [
                        {"label": "2026-07", "value": 2},
                        {"label": "2026-08", "value": 5},
                        {"label": "2026-09", "value": 4},
                        {"label": "2026-10", "value": 7}
                    ]
                })
                .to_string(),
                render_config: json!({
                    "tooltip": {"trigger": "axis"},
                    "grid": {"left": 40, "right": 20, "top": 32, "bottom": 32},
                    "xAxis": {"type": "category"},
                    "yAxis": {"type": "value"},
                    "dataset": {"source": []},
                    "series": [{"type": "bar", "encode": {"x": "label", "y": "value"}}]
                })
                .to_string(),
                refresh_cron: "0 0 4 * * * *".to_string(),
                status: chart_specs::STATUS_ACTIVE.to_string(),
            };
            let chart = chart_specs::Model::create_chart(&ctx.db, post.id, &params).await?;
            charts::refresh_chart(ctx, chart.id).await?;
        }

        crate::controllers::llms::invalidate(ctx).await;
        println!("demo content ready: /posts/hello-weft");
        Ok(())
    }
}
