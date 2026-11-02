(() => {
  const confirmForms = () => {
    document.querySelectorAll('form[data-confirm]').forEach((form) => {
      form.addEventListener('submit', (event) => {
        if (!window.confirm(form.dataset.confirm || '确定执行此操作？')) {
          event.preventDefault();
        }
      });
    });
  };

  const adminNav = () => {
    const links = Array.from(document.querySelectorAll('[data-admin-nav]'));
    if (!links.length) return;
    const path = window.location.pathname.replace(/\/$/, '') || '/admin';
    links.forEach((link) => {
      const href = link.getAttribute('href') || '';
      const active = path === '/admin/posts/new'
        ? href === '/admin/posts/new'
        : href === '/admin'
          ? path === '/admin'
          : path === href || path.startsWith(`${href}/`);
      link.classList.toggle('is-active', active);
    });
  };

  const categoryRegenerate = () => {
    const button = document.getElementById('regenerate-categories');
    if (!button) return;
    const status = document.getElementById('category-status');
    button.addEventListener('click', async () => {
      if (!window.confirm('重新分类会覆盖当前分类，继续？')) return;
      button.disabled = true;
      status.textContent = '正在分析全站文章并生成分类…';
      try {
        const response = await fetch('/admin/api/categories/regenerate', { method: 'POST', headers: { accept: 'application/json' } });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
        status.textContent = `完成：${payload.categories.length} 个分类，已归类 ${payload.assigned}/${payload.total} 篇`;
        window.setTimeout(() => window.location.reload(), 700);
      } catch (error) {
        status.textContent = `失败：${error.message}`;
        button.disabled = false;
      }
    });
  };

  const mediaLibrary = () => {
    const form = document.getElementById('media-upload-form');
    if (!form) return;
    const fileInput = document.getElementById('media-file');
    const status = document.getElementById('media-upload-status');
    form.addEventListener('submit', async (event) => {
      event.preventDefault();
      if (!fileInput?.files?.length) return;
      const data = new FormData();
      data.append('file', fileInput.files[0]);
      status.textContent = '上传中…';
      try {
        const response = await fetch('/admin/media/upload', { method: 'POST', body: data, headers: { accept: 'application/json' } });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
        window.location.reload();
      } catch (error) {
        status.textContent = `失败：${error.message}`;
      }
    });
  };

  const copyValues = () => {
    document.querySelectorAll('[data-copy-value]').forEach((button) => {
      button.addEventListener('click', async () => {
        const value = button.dataset.copyValue || '';
        try {
          await navigator.clipboard.writeText(value);
          const original = button.textContent;
          button.textContent = '已复制';
          window.setTimeout(() => { button.textContent = original; }, 1200);
        } catch {
          window.prompt('复制内容：', value);
        }
      });
    });
  };



  const siteHero = () => {
    const input = document.getElementById('site-hero');
    const file = document.getElementById('hero-image-file');
    const button = document.getElementById('hero-image-upload');
    const status = document.getElementById('hero-upload-status');
    const preview = document.getElementById('hero-image-preview');
    if (!input || !file || !button || !preview) return;
    const updatePreview = () => {
      preview.src = input.value.trim() || '/static/hero-banner.svg';
    };
    input.addEventListener('change', updatePreview);
    button.addEventListener('click', async () => {
      if (!file.files?.[0]) {
        status.textContent = '请先选择图片';
        return;
      }
      status.textContent = '上传中…';
      button.disabled = true;
      try {
        const data = new FormData();
        data.append('file', file.files[0]);
        const response = await fetch('/admin/media/upload', { method: 'POST', body: data, headers: { accept: 'application/json' } });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
        input.value = payload.url;
        updatePreview();
        status.textContent = '已上传并填入';
        file.value = '';
      } catch (error) {
        status.textContent = `失败：${error.message}`;
      } finally {
        button.disabled = false;
      }
    });
  };

  const mediaPicker = () => {
    const open = document.getElementById('open-media-picker');
    const modal = document.getElementById('media-picker-modal');
    if (!open || !modal) return;
    const search = document.getElementById('media-picker-search');
    const kind = document.getElementById('media-picker-kind');
    const results = document.getElementById('media-picker-results');
    const status = document.getElementById('media-picker-status');
    const closeButton = document.getElementById('media-picker-close');
    const cancel = document.getElementById('media-picker-cancel');
    const insert = document.getElementById('media-picker-insert');
    const cover = document.getElementById('media-picker-cover');
    const hero = document.getElementById('media-picker-hero');
    const body = document.getElementById('body_md');
    const heroInput = document.getElementById('site-hero');
    if (heroInput) {
      if (insert) insert.hidden = true;
      if (cover) cover.hidden = true;
      if (hero) hero.hidden = false;
    } else if (hero) {
      hero.hidden = true;
    }
    let selected = null;
    let timer = null;

    const escape = (value) => String(value ?? '').replace(/[&<>"']/g, (char) => ({
      '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
    }[char]));

    const close = () => {
      modal.hidden = true;
      document.body.style.overflow = '';
    };

    const render = (items) => {
      if (!items.length) {
        results.innerHTML = '<div class="empty-state"><p>没有找到媒体文件。</p></div>';
        return;
      }
      results.innerHTML = items.map((item) => {
        const thumb = item.kind === 'image'
          ? `<img src="${escape(item.url)}" alt="" loading="lazy" onerror="this.onerror=null;this.src='/static/default-cover.svg'">`
          : `<video src="${escape(item.url)}" preload="metadata" muted playsinline></video>`;
        return `<button type="button" class="media-pick-card" data-url="${escape(item.url)}" data-kind="${escape(item.kind)}" data-markdown="${escape(item.markdown)}" data-video-marker="${escape(item.video_marker)}"><span class="media-pick-thumb">${thumb}</span><span class="media-pick-name">${escape(item.original_name)}</span><span class="muted small">${escape(item.kind)} · ${Math.max(1, Math.round(item.size / 1024))} KB</span></button>`;
      }).join('');
      results.querySelectorAll('.media-pick-card').forEach((button) => {
        button.addEventListener('click', () => {
          selected = {
            url: button.dataset.url,
            kind: button.dataset.kind,
            markdown: button.dataset.markdown,
            videoMarker: button.dataset.videoMarker,
          };
          results.querySelectorAll('.media-pick-card').forEach((item) => item.classList.remove('is-selected'));
          button.classList.add('is-selected');
          status.textContent = '已选择 1 个文件';
        });
      });
    };

    const load = async () => {
      status.textContent = '加载中…';
      try {
        const params = new URLSearchParams();
        if (search?.value.trim()) params.set('q', search.value.trim());
        if (kind?.value) params.set('kind', kind.value);
        const response = await fetch(`/admin/api/media?${params.toString()}`, { headers: { accept: 'application/json' } });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
        render(payload.items || []);
        status.textContent = `${payload.items?.length || 0} 个文件`;
      } catch (error) {
        results.innerHTML = `<div class="alert error">加载失败：${escape(error.message)}</div>`;
        status.textContent = '';
      }
    };

    const insertBody = (marker) => {
      if (!body) return;
      const start = body.selectionStart ?? body.value.length;
      const end = body.selectionEnd ?? start;
      const text = `\n\n${marker}\n\n`;
      body.value = `${body.value.slice(0, start)}${text}${body.value.slice(end)}`;
      body.selectionStart = body.selectionEnd = start + text.length;
      body.focus();
    };

    open.addEventListener('click', () => {
      selected = null;
      status.textContent = '';
      modal.hidden = false;
      document.body.style.overflow = 'hidden';
      if (search) search.value = '';
      if (kind) kind.value = '';
      load();
      search?.focus();
    });
    closeButton?.addEventListener('click', close);
    cancel?.addEventListener('click', close);
    modal.addEventListener('click', (event) => { if (event.target === modal) close(); });
    modal.addEventListener('keydown', (event) => {
      if (event.key === 'Enter' && event.target instanceof HTMLInputElement) event.preventDefault();
    });
    window.addEventListener('keydown', (event) => { if (event.key === 'Escape' && !modal.hidden) close(); });
    search?.addEventListener('input', () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(load, 250);
    });
    kind?.addEventListener('change', load);
    insert?.addEventListener('click', () => {
      if (!selected) { status.textContent = '请先选择一个文件'; return; }
      insertBody(selected.kind === 'video' ? selected.videoMarker : selected.markdown);
      close();
      const postStatus = document.getElementById('post-media-status');
      if (postStatus) postStatus.textContent = '已从媒体库插入正文';
    });
    cover?.addEventListener('click', () => {
      if (!selected) { status.textContent = '请先选择一个文件'; return; }
      if (selected.kind !== 'image') { status.textContent = '视频不能设为封面'; return; }
      const input = document.getElementById('cover_image');
      if (input) input.value = selected.url;
      close();
      const postStatus = document.getElementById('post-media-status');
      if (postStatus) postStatus.textContent = '已从媒体库设为封面';
    });
    hero?.addEventListener('click', () => {
      if (!selected) { status.textContent = '请先选择一个文件'; return; }
      if (selected.kind !== 'image') { status.textContent = 'Banner 必须使用图片'; return; }
      if (heroInput) {
        heroInput.value = selected.url;
        heroInput.dispatchEvent(new Event('change', { bubbles: true }));
        const uploadStatus = document.getElementById('hero-upload-status');
        if (uploadStatus) uploadStatus.textContent = '已从媒体库选择';
      }
      close();
    });
  };

  const postMedia = () => {
    const fileInput = document.getElementById('post-media-file');
    const insertButton = document.getElementById('post-media-insert');
    const coverButton = document.getElementById('post-media-cover');
    const status = document.getElementById('post-media-status');
    const body = document.getElementById('body_md');
    if (!fileInput || !insertButton || !coverButton) return;

    const upload = async () => {
      const file = fileInput.files?.[0];
      if (!file) throw new Error('请选择文件');
      const data = new FormData();
      data.append('file', file);
      const response = await fetch('/admin/media/upload', { method: 'POST', body: data, headers: { accept: 'application/json' } });
      const payload = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
      return payload;
    };

    insertButton.addEventListener('click', async () => {
      status.textContent = '上传中…';
      try {
        const payload = await upload();
        const marker = `\n\n${payload.kind === 'image' ? payload.markdown : payload.video_marker}\n\n`;
        insertIntoBody(payload.kind === 'image' ? payload.markdown : payload.video_marker);
        status.textContent = '已插入正文';
        fileInput.value = '';
      } catch (error) {
        status.textContent = `失败：${error.message}`;
      }
    });

    const insertIntoBody = (marker) => {
      if (!body) return;
      const start = body.selectionStart ?? body.value.length;
      const end = body.selectionEnd ?? start;
      const text = `\n\n${marker}\n\n`;
      body.value = `${body.value.slice(0, start)}${text}${body.value.slice(end)}`;
      body.selectionStart = body.selectionEnd = start + text.length;
      body.focus();
    };

    coverButton.addEventListener('click', async () => {
      status.textContent = '上传中…';
      try {
        const payload = await upload();
        if (payload.kind !== 'image') throw new Error('视频不能设为封面');
        const cover = document.getElementById('cover_image');
        if (cover) cover.value = payload.url;
        status.textContent = '已设为封面';
        fileInput.value = '';
      } catch (error) {
        status.textContent = `失败：${error.message}`;
      }
    });
  };

  const articleEditor = () => {
    const run = document.getElementById('ai-edit-run');
    if (!run) return;
    const panel = document.querySelector('.ai-panel[data-post-id]');
    const body = document.getElementById('body_md');
    const title = document.getElementById('title');
    const modeSelect = document.getElementById('ai-edit-mode');
    const modeNote = document.getElementById('ai-edit-mode-note');
    const prompt = document.getElementById('ai-edit-prompt');
    const urls = document.getElementById('ai-edit-urls');
    const files = document.getElementById('ai-edit-files');
    const status = document.getElementById('ai-edit-status');
    const sidebarSources = document.getElementById('ai-edit-sources');
    const modal = document.getElementById('ai-diff-modal');
    const output = document.getElementById('ai-diff-output');
    const meta = document.getElementById('diff-meta');
    const metadata = document.getElementById('diff-metadata');
    const diffSources = document.getElementById('ai-diff-sources');
    const apply = document.getElementById('ai-diff-apply');
    const discard = document.getElementById('ai-diff-discard');
    let original = '';
    let proposal = '';
    let changed = false;
    let mode = 'split';
    let payloadState = null;

    const editMode = () => (modeSelect?.value === 'create' ? 'create' : 'edit');
    const syncMode = () => {
      const creating = editMode() === 'create';
      if (run) run.textContent = creating ? '从零创作' : '生成修改建议';
      if (modeNote) {
        modeNote.textContent = creating
          ? '正文可以留空；会先生成标题、摘要、标签和完整正文，再进入 diff。'
          : '对比后应用；修改现有正文不会自动改标题/摘要/标签。';
      }
    };
    if (body && !body.value.trim() && modeSelect) modeSelect.value = 'create';
    modeSelect?.addEventListener('change', syncMode);
    syncMode();

    const escapeHtml = (value) => String(value ?? '').replace(/[&<>"']/g, (character) => ({
      '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
    }[character]));

    const splitLines = (value) => String(value ?? '').replace(/\r\n/g, '\n').split('\n');

    const fallbackOps = (left, right) => [
      ...left.map((line) => ({ type: 'delete', line })),
      ...right.map((line) => ({ type: 'insert', line })),
    ];

    const buildDiff = (left, right) => {
      const n = left.length;
      const m = right.length;
      if (n === 0) return right.map((line) => ({ type: 'insert', line }));
      if (m === 0) return left.map((line) => ({ type: 'delete', line }));
      if (n * m > 1_500_000) return fallbackOps(left, right);
      const dp = Array.from({ length: n + 1 }, () => new Uint32Array(m + 1));
      for (let i = n - 1; i >= 0; i -= 1) {
        for (let j = m - 1; j >= 0; j -= 1) {
          dp[i][j] = left[i] === right[j]
            ? dp[i + 1][j + 1] + 1
            : Math.max(dp[i + 1][j], dp[i][j + 1]);
        }
      }
      const ops = [];
      let i = 0;
      let j = 0;
      while (i < n && j < m) {
        if (left[i] === right[j]) {
          ops.push({ type: 'equal', line: left[i] });
          i += 1;
          j += 1;
        } else if (dp[i + 1][j] >= dp[i][j + 1]) {
          ops.push({ type: 'delete', line: left[i] });
          i += 1;
        } else {
          ops.push({ type: 'insert', line: right[j] });
          j += 1;
        }
      }
      while (i < n) { ops.push({ type: 'delete', line: left[i] }); i += 1; }
      while (j < m) { ops.push({ type: 'insert', line: right[j] }); j += 1; }
      return ops;
    };

    const buildRows = (ops) => {
      const rows = [];
      let leftNo = 1;
      let rightNo = 1;
      let index = 0;
      while (index < ops.length) {
        if (ops[index].type === 'equal') {
          rows.push({
            type: 'equal', left: ops[index].line, right: ops[index].line,
            leftNo: leftNo++, rightNo: rightNo++,
          });
          index += 1;
          continue;
        }
        const deletes = [];
        const inserts = [];
        while (index < ops.length && ops[index].type !== 'equal') {
          if (ops[index].type === 'delete') deletes.push(ops[index].line);
          else inserts.push(ops[index].line);
          index += 1;
        }
        const count = Math.max(deletes.length, inserts.length);
        for (let k = 0; k < count; k += 1) {
          const left = k < deletes.length ? deletes[k] : null;
          const right = k < inserts.length ? inserts[k] : null;
          rows.push({
            type: left !== null && right !== null ? 'replace' : (left !== null ? 'delete' : 'insert'),
            left, right,
            leftNo: left !== null ? leftNo++ : null,
            rightNo: right !== null ? rightNo++ : null,
          });
        }
      }
      return rows;
    };

    const highlightPair = (left, right) => {
      if (left === null || right === null) {
        return {
          left: left === null ? '' : escapeHtml(left),
          right: right === null ? '' : escapeHtml(right),
        };
      }
      let prefix = 0;
      while (prefix < left.length && prefix < right.length && left[prefix] === right[prefix]) prefix += 1;
      let suffix = 0;
      while (
        suffix < left.length - prefix
        && suffix < right.length - prefix
        && left[left.length - 1 - suffix] === right[right.length - 1 - suffix]
      ) suffix += 1;
      const leftMid = left.slice(prefix, left.length - suffix);
      const rightMid = right.slice(prefix, right.length - suffix);
      const before = escapeHtml(left.slice(0, prefix));
      const after = escapeHtml(left.slice(left.length - suffix));
      return {
        left: `${before}<mark class="diff-del">${escapeHtml(leftMid)}</mark>${after}`,
        right: `${escapeHtml(right.slice(0, prefix))}<mark class="diff-add">${escapeHtml(rightMid)}</mark>${escapeHtml(right.slice(right.length - suffix))}`,
      };
    };

    const statsFor = (ops) => ({
      additions: ops.filter((op) => op.type === 'insert').length,
      deletions: ops.filter((op) => op.type === 'delete').length,
    });

    const renderSources = (sources, warnings) => {
      const items = [];
      (sources || []).forEach((source) => {
        items.push(`<a href="${escapeHtml(source.url)}" target="_blank" rel="noopener">${escapeHtml(source.title || source.url)}</a>`);
      });
      (warnings || []).forEach((warning) => {
        items.push(`<span class="warn">${escapeHtml(warning)}</span>`);
      });
      const html = items.join('');
      if (sidebarSources) {
        sidebarSources.hidden = !html;
        sidebarSources.innerHTML = html ? `<strong>参考：</strong>${html}` : '';
      }
      if (diffSources) {
        diffSources.innerHTML = html ? `<strong>参考：</strong>${html}` : '';
      }
    };

    const render = () => {
      if (!output) return;
      const left = splitLines(original);
      const right = splitLines(proposal);
      const ops = buildDiff(left, right);
      const rows = buildRows(ops);
      const stats = statsFor(ops);
      const metaText = `${payloadState?.mode === 'create' ? '从零创作' : '修改正文'} · ${payloadState?.offline ? '离线模式' : `${payloadState?.provider || 'AI'} / ${payloadState?.model || ''}`} · 会话 #${payloadState?.session_id || ''} · +${stats.additions} −${stats.deletions} 行`;
      if (meta) meta.textContent = metaText;
      if (metadata) {
        const bits = [];
        if (payloadState?.mode === 'create' && payloadState?.title) bits.push(`标题：${escapeHtml(payloadState.title)}`);
        if (payloadState?.mode === 'create' && payloadState?.summary) bits.push(`摘要：${escapeHtml(payloadState.summary)}`);
        if (payloadState?.mode === 'create' && Array.isArray(payloadState?.tags) && payloadState.tags.length) {
          bits.push(`标签：${escapeHtml(payloadState.tags.join('、'))}`);
        }
        metadata.hidden = bits.length === 0;
        metadata.innerHTML = bits.join('<br>');
      }
      if (apply) apply.disabled = !changed;
      document.querySelectorAll('[data-diff-mode]').forEach((button) => {
        button.classList.toggle('is-active', button.dataset.diffMode === mode);
      });
      if (!changed) {
        output.innerHTML = '<div class="diff-empty-note">模型没有产生差异。可以调整提示词或补充材料后重试。</div>';
        return;
      }
      if (mode === 'unified') {
        const html = rows.map((row) => {
          if (row.type === 'equal') {
            return `<div class="diff-line"><span class="diff-mark"> </span><span class="diff-text">${escapeHtml(row.left)}</span></div>`;
          }
          const parts = [];
          if (row.left !== null) {
            parts.push(`<div class="diff-line diff-del"><span class="diff-mark">−</span><span class="diff-text">${escapeHtml(row.left)}</span></div>`);
          }
          if (row.right !== null) {
            parts.push(`<div class="diff-line diff-add"><span class="diff-mark">+</span><span class="diff-text">${escapeHtml(row.right)}</span></div>`);
          }
          return parts.join('');
        }).join('');
        output.innerHTML = html;
        return;
      }
      const html = rows.map((row) => {
        const pair = highlightPair(row.left, row.right);
        const rowClass = row.type === 'replace' ? 'diff-replace' : `diff-${row.type}`;
        const leftClass = row.left === null ? 'diff-empty' : (row.type === 'insert' ? 'diff-empty' : 'diff-del');
        const rightClass = row.right === null ? 'diff-empty' : (row.type === 'delete' ? 'diff-empty' : 'diff-add');
        return `<tr class="${rowClass}">
          <td class="diff-num">${row.leftNo ?? ''}</td>
          <td class="diff-code diff-side ${leftClass}">${pair.left || '&nbsp;'}</td>
          <td class="diff-num">${row.rightNo ?? ''}</td>
          <td class="diff-code diff-side ${rightClass}">${pair.right || '&nbsp;'}</td>
        </tr>`;
      }).join('');
      output.innerHTML = `<table class="diff-table"><colgroup><col class="diff-num-col"><col><col class="diff-num-col"><col></colgroup><thead><tr><th colspan="2">原文</th><th colspan="2">建议</th></tr></thead><tbody>${html}</tbody></table>`;
    };

    const closeModal = () => {
      if (!modal) return;
      modal.hidden = true;
      document.body.style.overflow = '';
    };

    const openModal = (payload) => {
      payloadState = payload;
      original = payload.original ?? original;
      proposal = payload.proposed ?? proposal;
      changed = Boolean(payload.changed) || proposal.trim() !== original.trim();
      renderSources(payload.sources, payload.warnings);
      render();
      if (modal) {
        modal.hidden = false;
        document.body.style.overflow = 'hidden';
      }
    };

    run.addEventListener('click', async () => {
      const creating = editMode() === 'create';
      if (!body || (!creating && !body.value.trim())) {
        if (status) status.textContent = creating ? '正文为空，将直接创作' : '正文为空；请切换到“从零创作”';
        if (!body || !creating) return;
      }
      if (files && files.files.length > 5) {
        if (status) status.textContent = '一次最多上传 5 个材料文件';
        return;
      }
      const formData = new FormData();
      formData.append('post_id', Number(panel?.dataset.postId || 0) > 0 ? panel.dataset.postId : '');
      formData.append('mode', editMode());
      formData.append('title', title?.value || '');
      formData.append('prompt', prompt?.value || '');
      formData.append('body_md', body.value);
      formData.append('urls', urls?.value || '');
      if (files) Array.from(files.files).forEach((file) => formData.append('file', file));
      const originalLabel = run.textContent;
      run.disabled = true;
      run.textContent = '生成中…';
      if (status) status.textContent = creating
        ? '正在从零创作，解析材料/抓取网页并生成初稿…'
        : (files?.files.length || urls?.value.trim()
          ? '正在解析材料/抓取网页并生成修改建议…'
          : '正在生成修改建议…');
      try {
        const response = await fetch('/admin/api/ai/edit-article', {
          method: 'POST', body: formData, headers: { accept: 'application/json' },
        });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) {
          throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
        }
        original = payload.original || body.value;
        openModal(payload);
        if (status) {
          status.textContent = payload.changed
            ? `会话 #${payload.session_id} · ${payload.mode === 'create' ? '已生成初稿' : '已生成 diff'}，等待确认`
            : `会话 #${payload.session_id} · 没有产生改动`;
        }
      } catch (error) {
        if (status) status.textContent = `失败：${error.message}`;
      } finally {
        run.disabled = false;
        run.textContent = originalLabel;
      }
    });

    document.querySelectorAll('[data-diff-mode]').forEach((button) => {
      button.addEventListener('click', () => {
        mode = button.dataset.diffMode === 'unified' ? 'unified' : 'split';
        render();
      });
    });

    apply?.addEventListener('click', () => {
      if (!body || !changed) return;
      body.value = proposal;
      if (payloadState?.mode === 'create') {
        const titleInput = document.getElementById('title');
        const summaryInput = document.getElementById('summary');
        const tagsInput = document.getElementById('tags');
        if (titleInput && !titleInput.value.trim() && payloadState.title) titleInput.value = payloadState.title;
        if (summaryInput && !summaryInput.value.trim() && payloadState.summary) summaryInput.value = payloadState.summary;
        if (tagsInput && !tagsInput.value.trim() && Array.isArray(payloadState.tags) && payloadState.tags.length) {
          tagsInput.value = payloadState.tags.join(', ');
        }
      }
      body.focus();
      body.setSelectionRange(body.value.length, body.value.length);
      if (status) status.textContent = payloadState?.mode === 'create'
        ? '已应用 AI 初稿和元数据；请检查后点击“保存”'
        : '已应用到正文；请点击“保存”写入数据库';
      closeModal();
    });

    discard?.addEventListener('click', () => {
      if (status) status.textContent = '已丢弃修改建议';
      closeModal();
    });

    modal?.addEventListener('click', (event) => {
      if (event.target === modal) closeModal();
    });
    window.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && modal && !modal.hidden) closeModal();
    });
  };

  const refreshButtons = () => {
    document.querySelectorAll('[data-refresh-chart]').forEach((button) => {
      button.addEventListener('click', async () => {
        const id = button.dataset.refreshChart;
        button.disabled = true;
        const original = button.textContent;
        button.textContent = '刷新中…';
        try {
          await fetch(`/admin/charts/${id}/refresh`, { method: 'POST', headers: { accept: 'application/json' } });
          window.location.reload();
        } catch (error) {
          button.textContent = original;
          button.disabled = false;
          window.alert(`刷新失败：${error.message}`);
        }
      });
    });
  };

  const recipeSelector = () => {
    const select = document.getElementById('recipe');
    if (!select) return;
    const render = document.getElementById('render_config');
    const hint = document.getElementById('recipe-hint');
    select.addEventListener('change', () => {
      const option = select.selectedOptions[0];
      if (!option || !option.value) return;
      if (render && option.dataset.render) {
        try {
          render.value = JSON.stringify(JSON.parse(option.dataset.render), null, 2);
        } catch {
          render.value = option.dataset.render;
        }
      }
      if (hint) hint.textContent = option.dataset.prompt || '';
    });
  };

  const aiPreset = () => {
    const select = document.getElementById('ai-preset');
    if (!select) return;
    select.addEventListener('change', () => {
      const option = select.selectedOptions[0];
      if (!option || !option.value) return;
      const provider = option.dataset.provider || '';
      const baseUrl = option.dataset.baseUrl || '';
      const model = option.dataset.model || '';
      const needsKey = option.dataset.needsKey === 'true';
      const set = (id, value) => { const el = document.getElementById(id); if (el && value) el.value = value; };
      set('ai-provider', provider);
      set('ai-base-url', baseUrl);
      set('ai-model', model);
      const hint = document.getElementById('preset-hint');
      if (hint) hint.textContent = needsKey
        ? '该预设通常需要 API Key；Key 会保存在服务端，不会回显。'
        : '该预设通常不需要 API Key（本地服务请确认可达）。';
    });
  };

  const insertChartMarkers = () => {
    const body = document.getElementById('body_md');
    document.querySelectorAll('[data-insert-chart]').forEach((button) => {
      button.addEventListener('click', () => {
        if (!body) return;
        const id = button.dataset.insertChart;
        const marker = `\n\n[[chart:${id}]]\n\n`;
        const start = body.selectionStart ?? body.value.length;
        const end = body.selectionEnd ?? start;
        body.value = `${body.value.slice(0, start)}${marker}${body.value.slice(end)}`;
        const cursor = start + marker.length;
        body.selectionStart = cursor;
        body.selectionEnd = cursor;
        body.focus();
      });
    });
  };

  confirmForms();
  recipeSelector();
  refreshButtons();
  aiPreset();
  insertChartMarkers();
  articleEditor();
  mediaLibrary();
  copyValues();
  postMedia();
  mediaPicker();
  siteHero();
  categoryRegenerate();
  adminNav();
})();
