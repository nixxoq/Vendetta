pub const APP_JS: &str = r#"(function() {
  function initTheme() {
    const themeBtn = document.getElementById('theme-toggle');
    if (!themeBtn) return;
    
    function getEffectiveTheme() {
      const explicit = document.documentElement.getAttribute('data-theme') || (document.documentElement.dataset && document.documentElement.dataset.theme);
      if (explicit === 'dark' || explicit === 'light') return explicit;
      try {
        const saved = localStorage.getItem('vendetta-theme');
        if (saved === 'dark' || saved === 'light') return saved;
      } catch (e) {}
      return (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) ? 'dark' : 'light';
    }

    function updateIcon(theme) {
      const useEl = themeBtn.querySelector('use');
      if (useEl) {
        useEl.setAttribute('href', theme === 'dark' ? '#icon-sun' : '#icon-moon');
      }
      const label = theme === 'dark' ? 'Switch to Light theme' : 'Switch to Dark theme';
      themeBtn.setAttribute('title', label);
      themeBtn.setAttribute('aria-label', label);
    }

    const current = getEffectiveTheme();
    document.documentElement.setAttribute('data-theme', current);
    if (document.documentElement.dataset) {
      document.documentElement.dataset.theme = current;
    }
    updateIcon(current);

    themeBtn.addEventListener('click', () => {
      const cur = document.documentElement.getAttribute('data-theme') || (document.documentElement.dataset && document.documentElement.dataset.theme) || getEffectiveTheme();
      const next = cur === 'dark' ? 'light' : 'dark';
      document.documentElement.setAttribute('data-theme', next);
      if (document.documentElement.dataset) {
        document.documentElement.dataset.theme = next;
      }
      try {
        localStorage.setItem('vendetta-theme', next);
      } catch (e) {}
      updateIcon(next);
    });
  }

  function initSpoilers() {
    document.addEventListener('click', (e) => {
      const spoiler = e.target.closest('.tg-spoiler');
      if (spoiler) {
        spoiler.classList.toggle('revealed');
      }
    });
  }

  function highlightAnchor() {
    if (window.location.hash) {
      const el = document.getElementById(window.location.hash.substring(1));
      if (el) {
        el.classList.add('highlight-target');
        el.scrollIntoView({ behavior: 'smooth', block: 'center' });
        setTimeout(() => el.classList.remove('highlight-target'), 2500);
      }
    }
  }

  function initChatInfoModal() {
    const headerTitle = document.querySelector('.chat-title-info');
    const modal = document.getElementById('chat-info-modal');
    const closeBtn = document.getElementById('chat-info-close');
    if (!headerTitle || !modal) return;

    headerTitle.addEventListener('click', () => {
      modal.classList.add('open');
    });

    if (closeBtn) {
      closeBtn.addEventListener('click', () => {
        modal.classList.remove('open');
      });
    }

    modal.addEventListener('click', (e) => {
      if (e.target === modal) modal.classList.remove('open');
    });

    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && modal.classList.contains('open')) {
        modal.classList.remove('open');
      }
    });
  }

  function initBlockquotes() {
    document.addEventListener('click', (e) => {
      const bq = e.target.closest('.tg-blockquote-collapsed, .tg-blockquote[data-collapsed="true"]');
      if (bq) {
        bq.classList.toggle('expanded');
      }
    });
  }

  function initReactions() {
    document.addEventListener('click', (e) => {
      const badge = e.target.closest('.reaction-badge');
      if (badge) {
        const wasOpen = badge.classList.contains('popover-open');
        document.querySelectorAll('.reaction-badge.popover-open').forEach((el) => {
          el.classList.remove('popover-open');
        });
        if (!wasOpen) {
          badge.classList.add('popover-open');
        }
        return;
      }

      if (!e.target.closest('.reaction-popover')) {
        document.querySelectorAll('.reaction-badge.popover-open').forEach((el) => {
          el.classList.remove('popover-open');
        });
      }
    });

    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        document.querySelectorAll('.reaction-badge.popover-open').forEach((el) => {
          el.classList.remove('popover-open');
        });
      }
    });
  }

  function initTgsStickers() {
    const canvases = document.querySelectorAll('canvas.sticker-canvas[data-tgs-url]');
    canvases.forEach((canvas) => {
      const url = canvas.getAttribute('data-tgs-url');
      if (!url) return;
      if (typeof window.DecompressionStream !== 'undefined') {
        fetch(url)
          .then((res) => res.blob())
          .then((blob) => {
            const ds = new DecompressionStream('gzip');
            const decompressedStream = blob.stream().pipeThrough(ds);
            return new Response(decompressedStream).json();
          })
          .then((lottieData) => {
            const ctx = canvas.getContext('2d');
            if (ctx && lottieData && lottieData.nm) {
              canvas.title = lottieData.nm;
            }
          })
          .catch(() => {});
      }
    });
  }

  window.addEventListener('hashchange', highlightAnchor);
  window.addEventListener('DOMContentLoaded', () => {
    initTheme();
    initSpoilers();
    initBlockquotes();
    highlightAnchor();
    initChatInfoModal();
    initReactions();
    initTgsStickers();
  });
})();"#;

pub const LIGHTBOX_JS: &str = r#"(function() {
  function initLightbox() {
    const modal = document.createElement('div');
    modal.className = 'lightbox-modal';
    modal.id = 'lightbox-modal';
    modal.innerHTML = `
      <div class="lightbox-backdrop"></div>
      <div class="lightbox-content">
        <img src="" alt="Enlarged media" class="lightbox-img" id="lightbox-img">
        <button class="lightbox-close" aria-label="Close">&times;</button>
      </div>
    `;
    document.body.appendChild(modal);

    const img = modal.querySelector('#lightbox-img');
    const closeBtn = modal.querySelector('.lightbox-close');
    const backdrop = modal.querySelector('.lightbox-backdrop');

    function close() {
      modal.classList.remove('open');
      img.src = '';
    }

    closeBtn.addEventListener('click', close);
    backdrop.addEventListener('click', close);
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && modal.classList.contains('open')) close();
    });

    document.addEventListener('click', (e) => {
      const trigger = e.target.closest('.media-lightbox-trigger');
      if (trigger) {
        e.preventDefault();
        const fullSrc = trigger.getAttribute('data-full-src') || trigger.getAttribute('src');
        if (fullSrc) {
          img.src = fullSrc;
          modal.classList.add('open');
        }
      }
    });
  }

  window.addEventListener('DOMContentLoaded', initLightbox);
})();"#;

pub const SEARCH_JS: &str = r##"(function() {
  window.__VENDETTA_SEARCH_STORE__ = {
    shards: {},
    loadedShardIds: new Set(),
    shardAccessOrder: [],
    maxCachedShards: 10,
    manifest: null,
    loading: false
  };

  window.__VENDETTA_REGISTER_SEARCH_SHARD__ = function(shard) {
    if (shard && shard.shard_id) {
      const store = window.__VENDETTA_SEARCH_STORE__;
      store.shards[shard.shard_id] = shard;
      store.loadedShardIds.add(shard.shard_id);
      store.shardAccessOrder.push(shard.shard_id);
      evictOldShards();
    }
  };

  function evictOldShards() {
    const store = window.__VENDETTA_SEARCH_STORE__;
    while (store.loadedShardIds.size > store.maxCachedShards && store.shardAccessOrder.length > 0) {
      const oldestId = store.shardAccessOrder.shift();
      delete store.shards[oldestId];
      store.loadedShardIds.delete(oldestId);
      const el = document.getElementById('search-shard-script-' + oldestId);
      if (el && el.parentNode) {
        el.parentNode.removeChild(el);
      }
    }
  }

  function loadShardScript(shardMeta, basePath) {
    return new Promise((resolve, reject) => {
      const store = window.__VENDETTA_SEARCH_STORE__;
      if (store.loadedShardIds.has(shardMeta.shard_id) && store.shards[shardMeta.shard_id]) {
        return resolve(store.shards[shardMeta.shard_id]);
      }
      const oldEl = document.getElementById('search-shard-script-' + shardMeta.shard_id);
      if (oldEl && oldEl.parentNode) {
        oldEl.parentNode.removeChild(oldEl);
      }
      const script = document.createElement('script');
      script.id = 'search-shard-script-' + shardMeta.shard_id;
      script.src = basePath + 'search/shards/' + shardMeta.file_name;
      script.onload = () => {
        resolve(store.shards[shardMeta.shard_id]);
      };
      script.onerror = () => reject(new Error('Failed to load shard ' + shardMeta.file_name));
      document.head.appendChild(script);
    });
  }

  // Unicode-aware tokenizer supporting Latin, Cyrillic, Greek, Ukrainian, accented chars and numbers
  function tokenize(text) {
    if (!text) return [];
    const matches = text.toLowerCase().match(/[\p{L}\p{N}]+/gu);
    return matches || [];
  }

  function scoreEntry(query, entry) {
    const qTrim = query.trim().toLowerCase();
    if (!qTrim) return 1; // Base score if text query is empty but filters match

    const qTokens = tokenize(qTrim);
    if (!qTokens.length) return 0;

    let matched = 0;
    let exact = 0;
    for (let qt of qTokens) {
      for (let et of entry.tokens) {
        if (et === qt) {
          exact++;
          matched++;
          break;
        } else if (et.startsWith(qt)) {
          matched++;
          break;
        }
      }
    }

    if (matched === qTokens.length) {
      const textLower = entry.text.toLowerCase();
      if (textLower.includes(qTrim)) return 100;
      return exact === qTokens.length ? 50 : 20 + (exact * 5);
    }
    return 0;
  }

  function compareMatches(a, b) {
    if (b.score !== a.score) return b.score - a.score;
    if (b.entry.date !== a.entry.date) return b.entry.date - a.entry.date;
    if (a.entry.peer_id !== b.entry.peer_id) return a.entry.peer_id - b.entry.peer_id;
    return a.entry.msg_id - b.entry.msg_id;
  }

  function insertTopMatch(topMatches, item, limit) {
    if (topMatches.length >= limit) {
      const worst = topMatches[topMatches.length - 1];
      if (compareMatches(item, worst) >= 0) {
        return;
      }
    }
    let low = 0;
    let high = topMatches.length;
    while (low < high) {
      const mid = (low + high) >>> 1;
      if (compareMatches(item, topMatches[mid]) < 0) {
        high = mid;
      } else {
        low = mid + 1;
      }
    }
    topMatches.splice(low, 0, item);
    if (topMatches.length > limit) {
      topMatches.pop();
    }
  }

  function getCandidateShards(query, selPeer, dFrom, dTo) {
    const manifest = window.__VENDETTA_SEARCH_MANIFEST__;
    if (!manifest || !manifest.shards) return [];

    const qTokens = tokenize(query);
    let candidateIds = null;

    if (qTokens.length > 0 && manifest.prefix_index) {
      for (let qt of qTokens) {
        let tokenShardMatches = new Set();
        const chars = Array.from(qt);
        if (chars.length >= 3) {
          const p3 = chars.slice(0, 3).join('');
          if (manifest.prefix_index[p3]) {
            for (let sid of manifest.prefix_index[p3]) tokenShardMatches.add(sid);
          }
        } else if (chars.length === 2) {
          const p2 = chars.slice(0, 2).join('');
          if (manifest.prefix_index[p2]) {
            for (let sid of manifest.prefix_index[p2]) tokenShardMatches.add(sid);
          }
        } else if (chars.length === 1) {
          const p1 = chars.slice(0, 1).join('');
          if (manifest.prefix_index[p1]) {
            for (let sid of manifest.prefix_index[p1]) tokenShardMatches.add(sid);
          }
        }

        if (candidateIds === null) {
          candidateIds = tokenShardMatches;
        } else {
          let intersected = new Set();
          for (let sid of candidateIds) {
            if (tokenShardMatches.has(sid)) intersected.add(sid);
          }
          candidateIds = intersected;
        }
      }
    }

    if (candidateIds === null) {
      candidateIds = new Set(manifest.shards.map(s => s.shard_id));
    }

    let candidates = [];
    for (let s of manifest.shards) {
      if (!candidateIds.has(s.shard_id)) continue;
      if (selPeer !== null && s.peer_ids && s.peer_ids.length && !s.peer_ids.includes(selPeer)) {
        continue;
      }
      if (dFrom !== null && s.max_date && s.max_date < dFrom) {
        continue;
      }
      if (dTo !== null && s.min_date && s.min_date > dTo) {
        continue;
      }
      candidates.push(s);
    }
    return candidates;
  }

  // Export internals for test verification
  window.__VENDETTA_SEARCH_INTERNALS__ = {
    tokenize,
    scoreEntry,
    compareMatches,
    insertTopMatch,
    getCandidateShards
  };

  function initSearchUI() {
    const modal = document.getElementById('search-modal');
    const openBtn = document.getElementById('search-open-btn');
    const input = document.getElementById('search-input');
    const resultsContainer = document.getElementById('search-results-list');
    
    const peerFilter = document.getElementById('search-peer-filter');
    const senderFilter = document.getElementById('search-sender-filter');
    const dateFrom = document.getElementById('search-date-from');
    const dateTo = document.getElementById('search-date-to');
    const mediaFilter = document.getElementById('search-media-filter');
    const hasReply = document.getElementById('search-has-reply');
    const isEdited = document.getElementById('search-is-edited');
    const isDeleted = document.getElementById('search-is-deleted');
    const isForward = document.getElementById('search-is-forward');

    if (!modal || !openBtn || !input || !resultsContainer) return;

    const basePath = modal.getAttribute('data-base-path') || '';

    // Load search manifest
    if (!window.__VENDETTA_SEARCH_MANIFEST__) {
      const manScript = document.createElement('script');
      manScript.src = basePath + 'search/manifest.js';
      manScript.onload = () => {
        if (window.__VENDETTA_SEARCH_MANIFEST__ && peerFilter) {
          for (let p of window.__VENDETTA_SEARCH_MANIFEST__.peers) {
            const opt = document.createElement('option');
            opt.value = p.peer_id;
            opt.textContent = p.name;
            peerFilter.appendChild(opt);
          }
        }
      };
      document.head.appendChild(manScript);
    }

    openBtn.addEventListener('click', () => {
      modal.classList.add('open');
      input.focus();
    });

    modal.addEventListener('click', (e) => {
      if (e.target === modal) modal.classList.remove('open');
    });

    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && modal.classList.contains('open')) {
        modal.classList.remove('open');
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
        e.preventDefault();
        modal.classList.add('open');
        input.focus();
      }
    });

    async function performSearch() {
      const q = input.value.trim();
      const selPeer = peerFilter && peerFilter.value ? parseInt(peerFilter.value, 10) : null;
      const sFilter = senderFilter ? senderFilter.value.trim().toLowerCase() : '';
      const dFromSec = dateFrom && dateFrom.value ? Math.floor(new Date(dateFrom.value).getTime() / 1000) : null;
      const dToSec = dateTo && dateTo.value ? Math.floor(new Date(dateTo.value).getTime() / 1000) + 86400 : null;
      const mFilter = mediaFilter ? mediaFilter.value : '';
      const replyOnly = hasReply ? hasReply.checked : false;
      const editedOnly = isEdited ? isEdited.checked : false;
      const deletedOnly = isDeleted ? isDeleted.checked : false;
      const fwdOnly = isForward ? isForward.checked : false;

      resultsContainer.innerHTML = '';

      const hasAnyFilter = q || selPeer !== null || sFilter || dFromSec || dToSec || mFilter || replyOnly || editedOnly || deletedOnly || fwdOnly;
      if (!hasAnyFilter) {
        resultsContainer.innerHTML = '<li class="search-result-item text-muted">Type to search messages...</li>';
        return;
      }

      const candidateShards = getCandidateShards(q, selPeer, dFromSec, dToSec);
      if (!candidateShards.length) {
        resultsContainer.innerHTML = '<li class="search-result-item text-muted">No matching messages found</li>';
        return;
      }

      const LIMIT = 50;
      const topMatches = [];

      // Stream candidate shards one by one, processing entries immediately into bounded top-N collector
      for (let s of candidateShards) {
        let shard = null;
        try {
          shard = await loadShardScript(s, basePath);
        } catch (e) {
          console.warn('Shard load error:', e);
        }
        if (!shard || !shard.entries) continue;

        for (let entry of shard.entries) {
          if (selPeer !== null && entry.peer_id !== selPeer) continue;
          if (sFilter && !entry.sender.toLowerCase().includes(sFilter)) continue;
          if (dFromSec !== null && entry.date < dFromSec) continue;
          if (dToSec !== null && entry.date > dToSec) continue;
          if (mFilter && (!entry.media_types || !entry.media_types.includes(mFilter))) continue;
          if (replyOnly && !entry.is_reply) continue;
          if (editedOnly && entry.state !== 'edited') continue;
          if (deletedOnly && entry.state !== 'deleted') continue;
          if (fwdOnly && !entry.is_fwd) continue;

          const score = scoreEntry(q, entry);
          if (score > 0) {
            insertTopMatch(topMatches, { entry, score }, LIMIT);
          }
        }

        // Shard is processed, bounded cache eviction keeps memory strictly bounded
        evictOldShards();
      }

      if (!topMatches.length) {
        resultsContainer.innerHTML = '<li class="search-result-item text-muted">No matching messages found</li>';
        return;
      }

      const qTokens = tokenize(q);

      for (let i = 0; i < topMatches.length; i++) {
        const { entry } = topMatches[i];
        const li = document.createElement('li');
        li.className = 'search-result-item';
        const fullUrl = basePath + entry.url;
        const snippetHtml = extractSnippet(entry.text, qTokens);
        const dateStr = new Date(entry.date * 1000).toLocaleDateString(undefined, {
          year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit'
        });
        
        li.innerHTML = `
          <a href="${fullUrl}" class="search-result-link">
            <div class="search-res-meta">
              <span class="search-res-peer">${escapeHtml(entry.peer_name)}</span> &bull; <span class="search-res-sender">${escapeHtml(entry.sender)}</span> &bull; <span class="search-res-date">${dateStr}</span>
            </div>
            <div class="search-res-text">${snippetHtml}</div>
          </a>
        `;
        li.addEventListener('click', () => modal.classList.remove('open'));
        resultsContainer.appendChild(li);
      }
    }

    input.addEventListener('input', performSearch);
    if (peerFilter) peerFilter.addEventListener('change', performSearch);
    if (senderFilter) senderFilter.addEventListener('input', performSearch);
    if (dateFrom) dateFrom.addEventListener('change', performSearch);
    if (dateTo) dateTo.addEventListener('change', performSearch);
    if (mediaFilter) mediaFilter.addEventListener('change', performSearch);
    if (hasReply) hasReply.addEventListener('change', performSearch);
    if (isEdited) isEdited.addEventListener('change', performSearch);
    if (isDeleted) isDeleted.addEventListener('change', performSearch);
    if (isForward) isForward.addEventListener('change', performSearch);
  }

  function escapeRegex(string) {
    return string.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  function extractSnippet(text, queryTokens) {
    if (!text) return '<span class="text-muted">[Attachment]</span>';
    if (!queryTokens || queryTokens.length === 0) {
      const snippet = text.slice(0, 220) + (text.length > 220 ? ' ...' : '');
      return escapeHtml(snippet);
    }

    const lower = text.toLowerCase();
    let firstMatchIdx = -1;
    for (let t of queryTokens) {
      const idx = lower.indexOf(t.toLowerCase());
      if (idx !== -1 && (firstMatchIdx === -1 || idx < firstMatchIdx)) {
        firstMatchIdx = idx;
      }
    }

    if (firstMatchIdx === -1) {
      const snippet = text.slice(0, 220) + (text.length > 220 ? ' ...' : '');
      return escapeHtml(snippet);
    }

    const start = Math.max(0, firstMatchIdx - 50);
    const end = Math.min(text.length, firstMatchIdx + 170);
    const prefix = start > 0 ? '... ' : '';
    const suffix = end < text.length ? ' ...' : '';
    const chunk = text.slice(start, end);

    let escaped = escapeHtml(chunk);
    for (let t of queryTokens) {
      if (!t || t.length === 0) continue;
      const escToken = escapeHtml(t);
      const regex = new RegExp('(' + escapeRegex(escToken) + ')', 'gi');
      escaped = escaped.replace(regex, '<mark class="search-highlight">$1</mark>');
    }

    return prefix + escaped + suffix;
  }

  function escapeHtml(str) {
    return str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
  }

  // Close date-nav-dropdown when clicking outside
  document.addEventListener('click', function (e) {
    document.querySelectorAll('details.date-nav-dropdown[open]').forEach(function (el) {
      if (!el.contains(e.target)) {
        el.removeAttribute('open');
      }
    });
  });

  window.addEventListener('DOMContentLoaded', initSearchUI);
})();"##;
