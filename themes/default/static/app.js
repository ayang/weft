(() => {
  const escapeHtml = (value) => String(value)
    .replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;').replaceAll("'", '&#39;');

  const readingProgress = () => {
    const bar = document.getElementById('reading-progress');
    if (!bar) return;
    const article = document.querySelector('.article');
    if (!article) return;
    const update = () => {
      const rect = article.getBoundingClientRect();
      const total = article.offsetHeight - window.innerHeight;
      const progress = total <= 0 ? 100 : Math.min(100, Math.max(0, (-rect.top / total) * 100));
      bar.style.width = `${progress}%`;
    };
    update();
    window.addEventListener('scroll', update, { passive: true });
    window.addEventListener('resize', update);
  };

  const fallbackTable = (el, data, message) => {
    const rows = Array.isArray(data) ? data : (data && Array.isArray(data.rows) ? data.rows : null);
    el.innerHTML = '';
    if (!rows || rows.length === 0) {
      el.innerHTML = `<div class="chart-loading">${escapeHtml(message || '暂无图表数据')}</div>`;
      return;
    }
    const keys = Object.keys(rows[0]).slice(0, 8);
    const table = document.createElement('table');
    table.className = 'chart-fallback';
    const thead = document.createElement('thead');
    thead.innerHTML = `<tr>${keys.map((key) => `<th>${escapeHtml(key)}</th>`).join('')}</tr>`;
    const tbody = document.createElement('tbody');
    tbody.innerHTML = rows.slice(0, 50).map((row) =>
      `<tr>${keys.map((key) => `<td>${escapeHtml(row[key] ?? '')}</td>`).join('')}</tr>`
    ).join('');
    table.append(thead, tbody);
    el.append(table);
  };

  const numberFormatter = (format) => {
    if (!format || format === 'auto') return undefined;
    return (value) => {
      const number = Number(value);
      if (!Number.isFinite(number)) return value;
      if (format === 'integer') return String(Math.round(number));
      if (format === 'one_decimal') return number.toFixed(1);
      if (format === 'percent') return `${number.toFixed(1)}%`;
      if (format === 'compact') {
        return new Intl.NumberFormat('zh-CN', { notation: 'compact', maximumFractionDigits: 1 }).format(number);
      }
      return String(value);
    };
  };

  const applyFormatting = (option) => {
    const meta = option._weft || {};
    const formatter = numberFormatter(meta.number_format);
    if (!formatter) return;
    const axes = [];
    if (Array.isArray(option.yAxis)) axes.push(...option.yAxis);
    else if (option.yAxis) axes.push(option.yAxis);
    if (Array.isArray(option.xAxis)) axes.push(...option.xAxis);
    else if (option.xAxis && option.xAxis.type === 'value') axes.push(option.xAxis);
    axes.filter((axis) => axis.type === 'value' || axis.type === 'log').forEach((axis) => {
      axis.axisLabel = Object.assign({}, axis.axisLabel || {}, { formatter });
    });
    if (option.tooltip && option.tooltip.trigger === 'axis') {
      option.tooltip.valueFormatter = formatter;
    }
  };

  const applyData = (option, data) => {
    if (data == null) return option;
    if (option.dataset) {
      option.dataset.source = data;
      return option;
    }
    const hasSeriesData = Array.isArray(option.series)
      && option.series.some((series) => Array.isArray(series.data) && series.data.length > 0);
    if (hasSeriesData) return option;
    if (option.series && Array.isArray(data)) {
      if (data.length && typeof data[0] !== 'object') {
        option.series[0].data = data;
      } else {
        option.dataset = { source: data };
      }
    }
    return option;
  };

  const initCharts = async () => {
    const nodes = Array.from(document.querySelectorAll('[data-chart-id]'));
    if (nodes.length === 0) return;
    const dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    const textColor = dark ? '#d8d1c7' : '#3a332b';
    const lineColor = dark ? '#3a352f' : '#ddd4c6';
    for (const el of nodes) {
      const id = el.dataset.chartId;
      try {
        const endpoint = el.dataset.chartEndpoint || `/api/charts/${id}/data`;
        const response = await fetch(endpoint, { headers: { accept: 'application/json' } });
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        const payload = await response.json();
        let data = payload.data;
        if (data && !Array.isArray(data) && Array.isArray(data.rows)) data = data.rows;
        if (data == null) {
          fallbackTable(el, null, '数据正在后台刷新，请稍后刷新页面');
          continue;
        }
        if (!window.echarts) {
          fallbackTable(el, data, '图表库未加载，显示数据表');
          continue;
        }
        const option = applyData(payload.spec || {}, data);
        applyFormatting(option);
        option.backgroundColor = 'transparent';
        option.textStyle = Object.assign({ color: textColor }, option.textStyle || {});
        [].concat(option.xAxis || []).forEach((axis) => {
          axis.axisLine = Object.assign({ lineStyle: { color: lineColor } }, axis.axisLine || {});
          axis.axisLabel = Object.assign({}, axis.axisLabel || {}, { color: textColor });
        });
        [].concat(option.yAxis || []).forEach((axis) => {
          axis.splitLine = Object.assign({ lineStyle: { color: lineColor } }, axis.splitLine || {});
          axis.axisLabel = Object.assign({}, axis.axisLabel || {}, { color: textColor });
        });
        const chart = window.echarts.init(el, null, { renderer: 'canvas' });
        chart.setOption(option);
        if (payload.stale) {
          const note = document.createElement('div');
          note.className = 'muted small';
          note.style.textAlign = 'right';
          note.textContent = payload.updated_at ? `数据更新时间：${payload.updated_at}（后台刷新中）` : '数据等待首次刷新';
          el.after(note);
        }
        window.addEventListener('resize', () => chart.resize(), { passive: true });
      } catch (error) {
        fallbackTable(el, null, `图表加载失败：${error.message}`);
      }
    }
  };

  readingProgress();
  initCharts();
})();
