(() => {
  const root = document.getElementById('chart-builder');
  if (!root) return;
  let bootstrap = {};
  try {
    bootstrap = JSON.parse(root.dataset.bootstrap || '{}');
  } catch (error) {
    console.error('chart bootstrap parse failed', error);
    return;
  }

  const $ = (id) => document.getElementById(id);
  const form = $('chart-builder-form');
  const tableWrap = $('chart-table-wrap');
  const previewEl = $('chart-preview');
  const aiStatus = $('chart-ai-status');
  const previewStatus = $('preview-status');
  const previewMeta = $('preview-meta');
  let chart = null;
  let renderTimer = null;

  const palette = (name) => ({
    default: ['#5470c6', '#91cc75', '#fac858', '#ee6666', '#73c0de', '#3ba272'],
    warm: ['#a8442b', '#d98f70', '#e8c9a0', '#7b5e3b', '#c7a17a'],
    cool: ['#2f6f8f', '#5aa9a3', '#8ec9c0', '#6b8eaf', '#c9e4de'],
    mono: ['#3a332b', '#6f6a61', '#a49b8d', '#d8d1c7', '#8c8175'],
  }[name] || ['#5470c6']);

  const numeric = (value) => {
    if (typeof value === 'number') return Number.isFinite(value) ? value : 0;
    if (typeof value === 'string' && value.trim() !== '') {
      const number = Number(value);
      if (Number.isFinite(number)) return number;
    }
    return 0;
  };

  const state = {
    id: bootstrap.id || null,
    postId: bootstrap.post_id || 0,
    title: bootstrap.title || '',
    intent: bootstrap.intent || '',
    status: bootstrap.status || 'active',
    refreshCron: bootstrap.refresh_cron || '',
    sourceType: bootstrap.source_type || 'static',
    sqlQuery: bootstrap.sql_query || '',
    sqlLimit: bootstrap.sql_limit || 500,
    httpUrl: bootstrap.http_url || '',
    httpDataPath: bootstrap.http_data_path || '',
    kind: ['bar', 'line', 'area', 'pie', 'scatter'].includes(bootstrap.kind) ? bootstrap.kind : 'bar',
    xField: bootstrap.x_field || '',
    yFields: Array.isArray(bootstrap.y_fields) ? bootstrap.y_fields.slice() : [],
    options: Object.assign({
      smooth: false, stack: false, horizontal: false, legend: true, labels: false, palette: 'default', sort: 'none',
      height: 380, xLabel: '', yLabel: '', yUnit: '', numberFormat: 'auto',
      yMin: null, yMax: null, logY: false, dataZoom: false, legendPosition: 'top', dualAxis: false,
    }, bootstrap.options || {}, {
      xLabel: bootstrap.options?.x_label || '',
      yLabel: bootstrap.options?.y_label || '',
      yUnit: bootstrap.options?.y_unit || '',
      numberFormat: bootstrap.options?.number_format || 'auto',
      yMin: bootstrap.options?.y_min ?? null,
      yMax: bootstrap.options?.y_max ?? null,
      logY: Boolean(bootstrap.options?.log_y),
      dataZoom: Boolean(bootstrap.options?.data_zoom),
      legendPosition: bootstrap.options?.legend_position || 'top',
      dualAxis: Boolean(bootstrap.options?.dual_axis),
    }),
    columns: Array.isArray(bootstrap.columns) ? bootstrap.columns.slice() : [],
    rows: Array.isArray(bootstrap.rows) ? bootstrap.rows.slice() : [],
    previewRows: Array.isArray(bootstrap.rows) ? bootstrap.rows.slice() : [],
    advancedConfig: bootstrap.advanced_config || '{}',
    advancedRender: bootstrap.advanced_render || '{}',
  };

  const uniqueName = (base) => {
    let name = base;
    let index = 2;
    while (state.columns.includes(name)) {
      name = `${base} ${index++}`;
    }
    return name;
  };

  const fillForm = () => {
    $('title').value = state.title;
    $('intent').value = state.intent;
    $('post_id').value = String(state.postId || '');
    $('status').value = state.status;
    $('refresh_cron').value = state.refreshCron;
    $('data_source_type').value = state.sourceType;
    $('sql_query').value = state.sqlQuery;
    $('sql_limit').value = String(state.sqlLimit || 500);
    $('http_url').value = state.httpUrl;
    $('http_data_path').value = state.httpDataPath;
    $('data_source_config').value = state.advancedConfig;
    $('render_config').value = state.advancedRender;
  };

  const syncMetaFromForm = () => {
    state.title = $('title').value;
    state.intent = $('intent').value;
    state.postId = Number($('post_id').value) || 0;
    state.status = $('status').value;
    state.refreshCron = $('refresh_cron').value;
    state.sourceType = $('data_source_type').value;
    state.sqlQuery = $('sql_query').value;
    state.sqlLimit = Number($('sql_limit').value) || 500;
    state.httpUrl = $('http_url').value;
    state.httpDataPath = $('http_data_path').value;
    state.advancedConfig = $('data_source_config').value;
    state.advancedRender = $('render_config').value;
    state.kind = $('chart-kind').value;
    state.xField = $('x-field').value;
    state.options.palette = $('palette').value;
    state.options.sort = $('sort').value;
    state.options.smooth = $('opt-smooth').checked;
    state.options.stack = $('opt-stack').checked;
    state.options.horizontal = $('opt-horizontal').checked;
    state.options.legend = $('opt-legend').checked;
    state.options.labels = $('opt-labels').checked;
    state.options.height = Number($('opt-height').value) || 380;
    state.options.legendPosition = $('opt-legend-position').value;
    state.options.numberFormat = $('opt-number-format').value;
    state.options.xLabel = $('opt-x-label').value;
    state.options.yLabel = $('opt-y-label').value;
    state.options.yUnit = $('opt-y-unit').value;
    state.options.yMin = $('opt-y-min').value === '' ? null : Number($('opt-y-min').value);
    state.options.yMax = $('opt-y-max').value === '' ? null : Number($('opt-y-max').value);
    state.options.logY = $('opt-log-y').checked;
    state.options.dataZoom = $('opt-data-zoom').checked;
    state.options.dualAxis = $('opt-dual-axis').checked;
  };

  const sourceRows = () => state.sourceType === 'static' ? state.rows : state.previewRows;

  const setColumnValue = (row, column, value) => {
    state.rows[row][column] = value;
  };

  const renderTable = () => {
    const readOnly = state.sourceType !== 'static';
    if (state.columns.length === 0 || sourceRows().length === 0) {
      tableWrap.innerHTML = `<div class="table-empty">${readOnly ? '点击上方预览按钮拉取数据' : '还没有数据。上传文件、粘贴 CSV，或让 AI 生成示例数据。'}</div>`;
      $('table-stats').textContent = '0 行';
      return;
    }
    const rows = sourceRows();
    const header = state.columns.map((column, index) => `
      <th><div class="column-head">
        <input class="column-name" data-column="${index}" value="${escapeAttr(column)}" ${readOnly ? 'disabled' : ''}>
        ${readOnly ? '' : `<button type="button" class="column-delete" data-delete-column="${index}" title="删除列">×</button>`}
      </div></th>`).join('');
    const body = rows.slice(0, 1000).map((row, rowIndex) => {
      const cells = state.columns.map((column, columnIndex) => `
        <td><input class="cell-input" data-row="${rowIndex}" data-column="${columnIndex}" value="${escapeAttr(row[column] ?? '')}" ${readOnly ? 'disabled' : ''}></td>`).join('');
      return `<tr><th class="row-index">${rowIndex + 1}</th>${cells}${readOnly ? '' : `<td><button type="button" class="row-delete" data-delete-row="${rowIndex}" title="删除行">×</button></td>`}</tr>`;
    }).join('');
    tableWrap.innerHTML = `<table class="builder-table"><thead><tr><th class="row-index">#</th>${header}${readOnly ? '' : '<th></th>'}</tr></thead><tbody>${body}</tbody></table>`;
    $('table-stats').textContent = `${rows.length} 行 × ${state.columns.length} 列`;
    tableWrap.querySelectorAll('.cell-input').forEach((input) => {
      input.addEventListener('input', () => {
        const row = Number(input.dataset.row);
        const column = state.columns[Number(input.dataset.column)];
        if (!state.rows[row]) state.rows[row] = {};
        setColumnValue(row, column, input.value);
        schedulePreview();
      });
    });
    tableWrap.querySelectorAll('.column-name').forEach((input) => {
      input.addEventListener('change', () => {
        const index = Number(input.dataset.column);
        const oldName = state.columns[index];
        const newName = uniqueName(input.value.trim() || `列${index + 1}`);
        if (newName === oldName) return;
        state.columns[index] = newName;
        state.rows.forEach((row) => {
          row[newName] = row[oldName] ?? '';
          delete row[oldName];
        });
        if (state.xField === oldName) state.xField = newName;
        state.yFields = state.yFields.map((field) => field === oldName ? newName : field);
        renderTable();
        renderControls();
        schedulePreview();
      });
    });
    tableWrap.querySelectorAll('[data-delete-row]').forEach((button) => {
      button.addEventListener('click', () => {
        state.rows.splice(Number(button.dataset.deleteRow), 1);
        renderTable();
        schedulePreview();
      });
    });
    tableWrap.querySelectorAll('[data-delete-column]').forEach((button) => {
      button.addEventListener('click', () => {
        const column = state.columns[Number(button.dataset.deleteColumn)];
        state.columns = state.columns.filter((item) => item !== column);
        state.rows.forEach((row) => delete row[column]);
        state.yFields = state.yFields.filter((field) => field !== column);
        if (state.xField === column) state.xField = state.columns[0] || '';
        renderTable();
        renderControls();
        schedulePreview();
      });
    });
  };

  const renderSourcePanels = () => {
    $('source-static').hidden = state.sourceType !== 'static';
    $('source-sql').hidden = state.sourceType !== 'sql';
    $('source-http').hidden = state.sourceType !== 'http';
    $('source-note').textContent = state.sourceType === 'static'
      ? '静态数据可直接在表格内编辑；保存时会把表格写入图表数据。'
      : 'SQL/HTTP 数据由服务端拉取并缓存；表格只用于预览，保存后由后台按 cron 刷新。';
  };

  const renderControls = () => {
    const x = $('x-field');
    x.innerHTML = state.columns.map((column) => `<option value="${escapeAttr(column)}">${escapeText(column)}</option>`).join('');
    if (!state.columns.includes(state.xField)) state.xField = state.columns[0] || '';
    x.value = state.xField;
    const yWrap = $('y-fields');
    yWrap.innerHTML = state.columns.map((column) => `
      <label class="checkbox"><input type="checkbox" value="${escapeAttr(column)}" ${state.yFields.includes(column) ? 'checked' : ''} ${column === state.xField ? 'disabled' : ''}> ${escapeText(column)}</label>`).join('');
    yWrap.querySelectorAll('input[type=checkbox]').forEach((input) => {
      input.addEventListener('change', () => {
        state.yFields = Array.from(yWrap.querySelectorAll('input[type=checkbox]:checked')).map((item) => item.value);
        $('opt-dual-axis').disabled = state.yFields.length < 2 || state.kind === 'pie' || state.kind === 'scatter';
        schedulePreview();
      });
    });
    $('chart-kind').value = state.kind;
    $('palette').value = state.options.palette;
    $('sort').value = state.options.sort;
    $('opt-smooth').checked = Boolean(state.options.smooth);
    $('opt-stack').checked = Boolean(state.options.stack);
    $('opt-horizontal').checked = Boolean(state.options.horizontal);
    $('opt-legend').checked = Boolean(state.options.legend);
    $('opt-labels').checked = Boolean(state.options.labels);
    $('opt-height').value = String(state.options.height || 380);
    $('opt-legend-position').value = state.options.legendPosition || 'top';
    $('opt-number-format').value = state.options.numberFormat || 'auto';
    $('opt-x-label').value = state.options.xLabel || '';
    $('opt-y-label').value = state.options.yLabel || '';
    $('opt-y-unit').value = state.options.yUnit || '';
    $('opt-y-min').value = state.options.yMin == null ? '' : String(state.options.yMin);
    $('opt-y-max').value = state.options.yMax == null ? '' : String(state.options.yMax);
    $('opt-log-y').checked = Boolean(state.options.logY);
    $('opt-data-zoom').checked = Boolean(state.options.dataZoom);
    $('opt-dual-axis').checked = Boolean(state.options.dualAxis);
    $('opt-dual-axis').disabled = state.yFields.length < 2 || state.kind === 'pie' || state.kind === 'scatter';
    renderSourcePanels();
  };

  const sortedRows = (rows) => {
    if (state.options.sort === 'none') return rows.slice();
    const y = state.yFields[0] || '';
    return rows.slice().sort((left, right) => {
      const a = numeric(left[y] ?? left[state.xField]);
      const b = numeric(right[y] ?? right[state.xField]);
      return state.options.sort === 'desc' ? b - a : a - b;
    });
  };

  const axisName = (label, unit) => {
    if (!label && !unit) return '';
    if (label && unit) return `${label}（${unit}）`;
    return unit ? `（${unit}）` : label;
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

  const buildOption = (rows) => {
    const x = state.xField;
    const ys = state.yFields.length ? state.yFields : state.columns.filter((column) => column !== x).slice(0, 1);
    const kind = ['bar', 'line', 'area', 'pie', 'scatter'].includes(state.kind) ? state.kind : 'bar';
    const horizontal = state.options.horizontal && kind === 'bar';
    const data = rows.map((row) => {
      const copy = Object.assign({}, row);
      ys.forEach((field) => {
        if (copy[field] !== '' && copy[field] != null && Number.isFinite(Number(copy[field]))) {
          copy[field] = Number(copy[field]);
        }
      });
      return copy;
    });
    const formatter = numberFormatter(state.options.numberFormat);
    const legendPosition = state.options.legendPosition || 'top';
    let legend = { show: state.options.legend, top: 0, type: 'scroll' };
    let grid = {
      left: 48, right: 24,
      top: state.options.legend ? 38 : 16,
      bottom: 36, containLabel: true,
    };
    if (legendPosition === 'bottom') {
      legend = { show: state.options.legend, bottom: 0, type: 'scroll' };
      grid.top = state.options.legend ? 16 : 12;
      grid.bottom = state.options.dataZoom ? 64 : 52;
    } else if (legendPosition === 'right') {
      legend = { show: state.options.legend, right: 0, top: 8, orient: 'vertical', type: 'scroll' };
      grid.right = 100;
    }
    const option = {
      animationDuration: 420,
      backgroundColor: 'transparent',
      color: palette(state.options.palette),
      dataset: { source: data },
      tooltip: { trigger: kind === 'pie' ? 'item' : 'axis' },
      legend,
      grid,
    };
    if (formatter && kind !== 'pie') {
      option.tooltip.valueFormatter = formatter;
    }
    if (kind === 'pie') {
      option.legend = { show: state.options.legend, bottom: 0, type: 'scroll' };
      option.grid = null;
      option.xAxis = null;
      option.yAxis = null;
      option.series = [{
        type: 'pie',
        name: ys[0] || '值',
        radius: ['32%', '68%'],
        center: ['50%', '48%'],
        encode: { itemName: x, value: ys[0] },
        label: { show: state.options.labels, formatter: '{b}: {c}' },
      }];
      return option;
    }
    const xName = axisName(state.options.xLabel, '');
    const yName = axisName(state.options.yLabel, state.options.yUnit);
    const numericX = data.length > 0 && data.every((row) => row[x] !== '' && row[x] != null && Number.isFinite(Number(row[x])));
    option.xAxis = horizontal
      ? { type: 'value', name: yName }
      : { type: kind === 'scatter' && numericX ? 'value' : 'category', name: xName };
    let yAxis = horizontal
      ? { type: 'category', name: xName }
      : { type: state.options.logY ? 'log' : 'value', name: yName, nameGap: 42 };
    if (!horizontal) {
      if (state.options.yMin != null && state.options.yMin !== '') yAxis.min = Number(state.options.yMin);
      if (state.options.yMax != null && state.options.yMax !== '') yAxis.max = Number(state.options.yMax);
    }
    if (formatter) {
      if (option.xAxis.type === 'value') option.xAxis.axisLabel = { formatter };
      if (yAxis.type === 'value' || yAxis.type === 'log') yAxis.axisLabel = { formatter };
    }
    const dual = Boolean(state.options.dualAxis) && !horizontal && kind !== 'scatter' && ys.length >= 2;
    if (dual) {
      const right = Object.assign({}, yAxis, { position: 'right' });
      if (formatter && (right.type === 'value' || right.type === 'log')) right.axisLabel = { formatter };
      option.yAxis = [yAxis, right];
      grid.right = 64;
    } else {
      option.yAxis = yAxis;
    }
    if (state.options.dataZoom) {
      grid.bottom = legendPosition === 'bottom' ? 72 : 56;
      option.dataZoom = [{ type: 'inside' }, { type: 'slider', height: 18, bottom: 0 }];
    }
    option.grid = grid;
    option.series = ys.map((field, index) => {
      const item = {
        name: field,
        type: kind === 'area' ? 'line' : kind,
        encode: horizontal ? { x: field, y: x } : { x, y: field },
      };
      if (dual && index > 0) item.yAxisIndex = 1;
      if (kind === 'line' || kind === 'area') {
        item.smooth = Boolean(state.options.smooth);
        item.showSymbol = false;
      }
      if (kind === 'area') item.areaStyle = { opacity: 0.22 };
      if (state.options.stack && ['bar', 'line', 'area'].includes(kind)) item.stack = 'total';
      if (state.options.labels) item.label = { show: true };
      if (kind === 'scatter') item.symbolSize = 10;
      return item;
    });
    return option;
  };


  const advancedRows = () => {
    try {
      const config = JSON.parse($('data_source_config').value || '{}');
      if (Array.isArray(config.data)) return config.data;
      if (Array.isArray(config.rows)) return config.rows;
    } catch {
      // The config JSON error is reported by the advanced preview renderer.
    }
    return sourceRows();
  };

  const renderAdvancedPreview = () => {
    let spec;
    try {
      spec = JSON.parse($('render_config').value || '{}');
      if (!spec || typeof spec !== 'object' || Array.isArray(spec)) {
        throw new Error('render_config 必须是一个 JSON 对象');
      }
    } catch (error) {
      previewStatus.textContent = `高级 JSON 解析失败：${error.message}`;
      return;
    }
    const data = advancedRows();
    if (data && data.length) {
      const hasDataset = Boolean(spec.dataset);
      const hasSeriesData = Array.isArray(spec.series) && spec.series.some((series) => Array.isArray(series.data) && series.data.length > 0);
      if (hasDataset) {
        spec.dataset = Object.assign({}, spec.dataset, { source: data });
      } else if (!hasSeriesData) {
        spec.dataset = { source: data };
      }
    }
    if (!window.echarts) {
      previewEl.innerHTML = '<div class="chart-loading">ECharts 未加载，暂时无法预览。</div>';
      return;
    }
    previewEl.style.height = `${Number(state.options.height) || 380}px`;
    previewEl.style.minHeight = '0';
    if (chart) chart.resize();
    if (!chart) {
      previewEl.innerHTML = '';
      chart = window.echarts.init(previewEl, null, { renderer: 'canvas' });
      window.addEventListener('resize', () => chart && chart.resize(), { passive: true });
    }
    const dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    spec.textStyle = Object.assign({ color: dark ? '#d8d1c7' : '#3a332b' }, spec.textStyle || {});
    spec.backgroundColor = spec.backgroundColor || 'transparent';
    chart.setOption(spec, true);
    previewStatus.textContent = '高级 JSON 实时预览（数据从 data_source_config 注入 dataset，若已自带 series.data 则保留）。';
    previewMeta.textContent = `${data.length} 行 · advanced`;
  };

  const renderPreview = () => {
    if ($('advanced-mode').checked) {
      renderAdvancedPreview();
      return;
    }
    const rows = sortedRows(sourceRows());
    const ys = state.yFields.length ? state.yFields : state.columns.filter((column) => column !== state.xField).slice(0, 1);
    if (!rows.length || !state.xField || !ys.length) {
      if (chart) { chart.dispose(); chart = null; }
      previewEl.innerHTML = '<div class="chart-loading">添加数据并选择 X/Y 字段后显示预览</div>';
      previewStatus.textContent = '当前没有可绘制的数据。';
      previewMeta.textContent = '—';
      return;
    }
    if (!window.echarts) {
      previewEl.innerHTML = '<div class="chart-loading">ECharts 未加载，暂时无法预览；保存后仍会生成图表。</div>';
      return;
    }
    if (!chart) {
      previewEl.innerHTML = '';
      chart = window.echarts.init(previewEl, null, { renderer: 'canvas' });
      window.addEventListener('resize', () => chart && chart.resize(), { passive: true });
    }
    previewEl.style.height = `${Number(state.options.height) || 380}px`;
    previewEl.style.minHeight = '0';
    if (chart) chart.resize();
    const option = buildOption(rows);
    const dark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    const color = dark ? '#d8d1c7' : '#3a332b';
    option.textStyle = { color };
    chart.setOption(option, true);
    previewStatus.textContent = '';
    previewMeta.textContent = `${rows.length} 行 × ${state.columns.length} 列`;
  };

  const schedulePreview = () => {
    syncMetaFromForm();
    if (renderTimer) window.clearTimeout(renderTimer);
    renderTimer = window.setTimeout(renderPreview, 120);
  };

  const renderAll = () => {
    renderTable();
    renderControls();
    renderPreview();
  };

  const escapeHtml = (value) => String(value)
    .replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;').replaceAll("'", '&#39;');
  const escapeAttr = escapeHtml;
  const escapeText = escapeHtml;

  const parseDelimited = (text) => {
    const trimmed = text.trim();
    if (!trimmed) return null;
    if (trimmed.startsWith('[') || trimmed.startsWith('{')) {
      try {
        const value = JSON.parse(trimmed);
        const rows = Array.isArray(value) ? value : (value.rows || value.data || []);
        if (!Array.isArray(rows) || rows.length === 0) return null;
        if (rows.every((row) => row && typeof row === 'object' && !Array.isArray(row))) {
          const columns = [];
          rows.forEach((row) => Object.keys(row).forEach((key) => { if (!columns.includes(key)) columns.push(key); }));
          return { columns, rows };
        }
      } catch (error) {
        // fall through to CSV
      }
    }
    const delimiter = trimmed.split('\n')[0].includes('\t') ? '\t' : ',';
    const parsedRows = [];
    let row = [];
    let cell = '';
    let quoted = false;
    for (let index = 0; index < trimmed.length; index += 1) {
      const char = trimmed[index];
      const next = trimmed[index + 1];
      if (quoted && char === '"' && next === '"') {
        cell += '"';
        index += 1;
      } else if (char === '"') {
        quoted = !quoted;
      } else if (char === delimiter && !quoted) {
        row.push(cell);
        cell = '';
      } else if ((char === '\n' || char === '\r') && !quoted) {
        if (char === '\r' && next === '\n') index += 1;
        row.push(cell);
        if (row.some((value) => value.trim() !== '')) parsedRows.push(row);
        row = [];
        cell = '';
      } else {
        cell += char;
      }
    }
    row.push(cell);
    if (row.some((value) => value.trim() !== '')) parsedRows.push(row);
    if (parsedRows.length < 2) return null;
    const columns = parsedRows[0].map((header, index) => header.trim() || `列${index + 1}`);
    const rows = parsedRows.slice(1).map((values) => {
      const object = {};
      columns.forEach((column, index) => { object[column] = values[index] ?? ''; });
      return object;
    });
    return { columns, rows };
  };

  const applyBlueprint = (blueprint, keepSource = true) => {
    if (!blueprint || typeof blueprint !== 'object') return;
    if (!keepSource) state.sourceType = 'static';
    state.kind = blueprint.kind || state.kind;
    if (!['bar', 'line', 'area', 'pie', 'scatter'].includes(state.kind)) state.kind = 'bar';
    state.xField = blueprint.x_field || state.xField;
    state.yFields = Array.isArray(blueprint.y_fields) ? blueprint.y_fields.slice() : state.yFields;
    const incoming = blueprint.options || {};
    state.options = Object.assign(state.options, incoming, {
      xLabel: incoming.x_label ?? incoming.xLabel ?? state.options.xLabel,
      yLabel: incoming.y_label ?? incoming.yLabel ?? state.options.yLabel,
      yUnit: incoming.y_unit ?? incoming.yUnit ?? state.options.yUnit,
      numberFormat: incoming.number_format ?? incoming.numberFormat ?? state.options.numberFormat,
      yMin: incoming.y_min ?? incoming.yMin ?? state.options.yMin,
      yMax: incoming.y_max ?? incoming.yMax ?? state.options.yMax,
      logY: incoming.log_y ?? incoming.logY ?? state.options.logY,
      dataZoom: incoming.data_zoom ?? incoming.dataZoom ?? state.options.dataZoom,
      legendPosition: incoming.legend_position ?? incoming.legendPosition ?? state.options.legendPosition,
      dualAxis: incoming.dual_axis ?? incoming.dualAxis ?? state.options.dualAxis,
    });
    if (Array.isArray(blueprint.columns) && blueprint.columns.length) state.columns = blueprint.columns.slice();
    if (Array.isArray(blueprint.rows) && blueprint.rows.length) {
      state.rows = blueprint.rows.map((row) => Object.assign({}, row));
      if (state.sourceType !== 'static') state.previewRows = state.rows.slice();
      if (!state.columns.length) {
        const columns = [];
        state.rows.forEach((row) => Object.keys(row).forEach((key) => { if (!columns.includes(key)) columns.push(key); }));
        state.columns = columns;
      }
    }
    if (blueprint.title) state.title = blueprint.title;
    fillForm();
    renderAll();
  };

  const serverOptions = () => ({
    smooth: Boolean(state.options.smooth),
    stack: Boolean(state.options.stack),
    horizontal: Boolean(state.options.horizontal),
    legend: Boolean(state.options.legend),
    labels: Boolean(state.options.labels),
    palette: state.options.palette || 'default',
    sort: state.options.sort || 'none',
    height: Number(state.options.height) || 380,
    x_label: state.options.xLabel || '',
    y_label: state.options.yLabel || '',
    y_unit: state.options.yUnit || '',
    number_format: state.options.numberFormat || 'auto',
    y_min: state.options.yMin == null || Number.isNaN(Number(state.options.yMin)) ? null : Number(state.options.yMin),
    y_max: state.options.yMax == null || Number.isNaN(Number(state.options.yMax)) ? null : Number(state.options.yMax),
    log_y: Boolean(state.options.logY),
    data_zoom: Boolean(state.options.dataZoom),
    legend_position: state.options.legendPosition || 'top',
    dual_axis: Boolean(state.options.dualAxis),
  });

  const buildCurrentBlueprint = () => ({
    title: state.title,
    kind: state.kind,
    x_field: state.xField,
    y_fields: state.yFields,
    options: serverOptions(),
    columns: state.columns,
    rows: sourceRows(),
  });

  const aiAssist = async () => {
    const prompt = $('chart-prompt').value.trim();
    const file = $('chart-file').files[0];
    if (!prompt && !file) {
      aiStatus.textContent = '请输入提示词或选择一个文件。';
      return;
    }
    const button = $('chart-ai-generate');
    button.disabled = true;
    aiStatus.textContent = 'AI 正在解析并生成图表…';
    const data = new FormData();
    data.append('prompt', prompt);
    data.append('current', JSON.stringify(buildCurrentBlueprint()));
    if (file) data.append('file', file);
    try {
      const response = await fetch('/admin/api/charts/assist', { method: 'POST', body: data, headers: { accept: 'application/json' } });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
      applyBlueprint(payload.blueprint, state.sourceType !== 'static');
      aiStatus.textContent = `完成 · 会话 #${payload.session_id} · ${payload.offline ? '离线模式' : payload.model}${payload.file?.name ? ` · 文件 ${payload.file.name}` : ''}`;
    } catch (error) {
      aiStatus.textContent = `失败：${error.message}`;
    } finally {
      button.disabled = false;
    }
  };

  const previewSource = async (type) => {
    const status = type === 'sql' ? $('sql-preview-status') : $('http-preview-status');
    status.textContent = '正在拉取数据…';
    const config = type === 'sql'
      ? { query: $('sql_query').value, limit: Number($('sql_limit').value) || 500 }
      : { url: $('http_url').value, data_path: $('http_data_path').value };
    try {
      const response = await fetch('/admin/api/charts/preview', {
        method: 'POST',
        headers: { 'content-type': 'application/json', accept: 'application/json' },
        body: JSON.stringify({ data_source_type: type, data_source_config: config }),
      });
      const payload = await response.json();
      if (!response.ok) throw new Error(payload.message || payload.description || payload.error || `HTTP ${response.status}`);
      state.previewRows = payload.rows || [];
      if (payload.columns?.length) state.columns = payload.columns;
      if (!state.columns.includes(state.xField)) state.xField = state.columns[0] || '';
      if (state.yFields.length === 0) {
        state.yFields = state.columns.filter((column) => column !== state.xField).slice(0, 1);
      }
      renderTable();
      renderControls();
      renderPreview();
      status.textContent = `已拉取 ${state.previewRows.length} 行；保存后由后台刷新。`;
    } catch (error) {
      status.textContent = `拉取失败：${error.message}`;
    }
  };

  const syncForm = () => {
    syncMetaFromForm();
    $('kind_hidden').value = state.kind;
    $('source_hidden').value = state.sourceType;
    $('visual_mode').value = $('advanced-mode').checked ? '0' : '1';
    $('visual_config').value = JSON.stringify({
      kind: state.kind,
      x_field: state.xField,
      y_fields: state.yFields,
      options: serverOptions(),
      columns: state.columns,
      rows: sourceRows(),
      source_type: state.sourceType,
      sql_query: state.sqlQuery,
      sql_limit: state.sqlLimit,
      http_url: state.httpUrl,
      http_data_path: state.httpDataPath,
    });
  };

  const bind = () => {
    $('title').addEventListener('input', () => { state.title = $('title').value; });
    $('intent').addEventListener('input', () => { state.intent = $('intent').value; });
    $('chart-kind').addEventListener('change', () => { state.kind = $('chart-kind').value; renderControls(); schedulePreview(); });
    $('x-field').addEventListener('change', () => {
      state.xField = $('x-field').value;
      state.yFields = state.yFields.filter((field) => field !== state.xField);
      renderControls();
      schedulePreview();
    });
    ['palette', 'sort', 'opt-smooth', 'opt-stack', 'opt-horizontal', 'opt-legend', 'opt-labels'].forEach((id) => {
      $(id).addEventListener('change', schedulePreview);
    });
    $('data_source_type').addEventListener('change', () => {
      state.sourceType = $('data_source_type').value;
      renderSourcePanels();
      renderTable();
      renderControls();
      if (state.sourceType !== 'static') {
        previewSource(state.sourceType);
      } else {
        renderPreview();
      }
    });
    $('sql_query').addEventListener('input', () => { state.sqlQuery = $('sql_query').value; });
    $('sql_limit').addEventListener('input', () => { state.sqlLimit = Number($('sql_limit').value) || 500; });
    $('http_url').addEventListener('input', () => { state.httpUrl = $('http_url').value; });
    $('http_data_path').addEventListener('input', () => { state.httpDataPath = $('http_data_path').value; });
    $('preview-sql').addEventListener('click', () => previewSource('sql'));
    $('preview-http').addEventListener('click', () => previewSource('http'));
    $('chart-ai-generate').addEventListener('click', aiAssist);
    $('chart-file').addEventListener('change', () => {
      const file = $('chart-file').files[0];
      aiStatus.textContent = file ? `已选择：${file.name}` : '';
    });
    $('add-row').addEventListener('click', () => {
      if (state.columns.length === 0) state.columns = ['类别', '数值'];
      const row = {};
      state.columns.forEach((column) => { row[column] = ''; });
      state.rows.push(row);
      renderTable();
      schedulePreview();
    });
    $('add-column').addEventListener('click', () => {
      const name = uniqueName(`列${state.columns.length + 1}`);
      state.columns.push(name);
      state.rows.forEach((row) => { row[name] = ''; });
      renderTable();
      renderControls();
      schedulePreview();
    });
    $('reset-data').addEventListener('click', () => {
      if (!window.confirm('清空全部数据？')) return;
      state.rows = [];
      state.columns = [];
      state.previewRows = [];
      renderTable();
      renderControls();
      renderPreview();
    });
    $('import-paste').addEventListener('click', () => {
      const parsed = parseDelimited($('paste-data').value);
      if (!parsed) {
        aiStatus.textContent = '粘贴内容无法解析，请检查第一行表头与分隔符。';
        return;
      }
      state.columns = parsed.columns;
      state.rows = parsed.rows;
      state.xField = state.columns[0] || '';
      state.yFields = state.columns.slice(1, 2);
      renderAll();
    });
    document.querySelectorAll('[data-recipe-prompt]').forEach((button) => {
      button.addEventListener('click', () => {
        $('chart-prompt').value = button.dataset.recipePrompt || '';
        $('chart-prompt').focus();
      });
    });
    ['opt-height', 'opt-x-label', 'opt-y-label', 'opt-y-unit', 'opt-y-min', 'opt-y-max'].forEach((id) => {
      $(id).addEventListener('input', () => { syncMetaFromForm(); schedulePreview(); });
    });
    ['opt-legend-position', 'opt-number-format'].forEach((id) => {
      $(id).addEventListener('change', () => { syncMetaFromForm(); schedulePreview(); });
    });
    ['opt-log-y', 'opt-data-zoom', 'opt-dual-axis'].forEach((id) => {
      $(id).addEventListener('change', () => { syncMetaFromForm(); schedulePreview(); });
    });
    $('advanced-mode').addEventListener('change', () => {
      const advanced = $('advanced-mode').checked;
      $('advanced-panel').hidden = !advanced;
      $('visual_mode').value = advanced ? '0' : '1';
      if (!advanced) {
        $('data_source_config').value = state.advancedConfig;
        $('render_config').value = state.advancedRender;
      }
      schedulePreview();
    });
    $('render_config').addEventListener('input', schedulePreview);
    $('data_source_config').addEventListener('input', schedulePreview);
    form.addEventListener('submit', (event) => {
      if ($('advanced-mode').checked) {
        syncMetaFromForm();
        $('source_hidden').value = state.sourceType;
        $('kind_hidden').value = state.kind;
        $('visual_mode').value = '0';
        return;
      }
      syncForm();
      if ($('visual_config').value.length > 2 * 1024 * 1024) {
        event.preventDefault();
        aiStatus.textContent = '数据超过 2 MiB，请减少行数或列数。';
        return;
      }
    });
  };

  document.querySelectorAll('[data-copy-marker]').forEach((button) => {
    button.addEventListener('click', async () => {
      const marker = document.getElementById('chart-marker')?.textContent || '';
      try {
        await navigator.clipboard.writeText(marker);
        button.textContent = '已复制';
        window.setTimeout(() => { button.textContent = '复制'; }, 1500);
      } catch {
        window.prompt('复制这个插入标记：', marker);
      }
    });
  });

  fillForm();
  renderControls();
  renderTable();
  bind();
  renderPreview();
  if (state.sourceType !== 'static') {
    window.setTimeout(() => previewSource(state.sourceType), 300);
  }
})();
