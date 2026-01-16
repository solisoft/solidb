/**
 * SDBQL Documentation Search - Vanilla JS
 */

(function() {
  var searchData = null;
  var searchIndex = [];
  var selectedIndex = -1;
  var currentResults = [];

  // Load search data on page load
  fetch('/public/data/sdbql-methods.json')
    .then(function(r) { return r.json(); })
    .then(function(data) {
      searchData = data;
      buildIndex();
    })
    .catch(function(e) {
      console.error('Failed to load search data:', e);
    });

  function buildIndex() {
    searchIndex = [];

    // Add functions
    (searchData.functions || []).forEach(function(fn) {
      searchIndex.push({
        type: 'function',
        name: fn.name,
        nameLower: fn.name.toLowerCase(),
        description: fn.description,
        descLower: fn.description.toLowerCase(),
        category: fn.category,
        url: fn.url
      });
    });

    // Add operators
    (searchData.operators || []).forEach(function(op) {
      searchIndex.push({
        type: 'operator',
        name: op.name,
        nameLower: op.name.toLowerCase(),
        description: op.description,
        descLower: op.description.toLowerCase(),
        category: op.category,
        url: op.url
      });
    });

    // Add keywords
    (searchData.keywords || []).forEach(function(kw) {
      searchIndex.push({
        type: 'keyword',
        name: kw.name,
        nameLower: kw.name.toLowerCase(),
        description: kw.description,
        descLower: kw.description.toLowerCase(),
        category: kw.category,
        url: kw.url
      });
    });
  }

  function search(query) {
    if (!query || !searchIndex.length) return [];

    var q = query.trim().toLowerCase();
    if (!q) return [];

    var results = [];

    for (var i = 0; i < searchIndex.length; i++) {
      var item = searchIndex[i];
      var score = 0;

      // Exact match
      if (item.nameLower === q) {
        score = 1000;
      }
      // Starts with
      else if (item.nameLower.indexOf(q) === 0) {
        score = 800;
      }
      // Contains in name
      else if (item.nameLower.indexOf(q) !== -1) {
        score = 600;
      }
      // Contains in description
      else if (item.descLower.indexOf(q) !== -1) {
        score = 200;
      }

      if (score > 0) {
        results.push({ item: item, score: score });
      }
    }

    // Sort by score, then alphabetically
    results.sort(function(a, b) {
      if (b.score !== a.score) return b.score - a.score;
      return a.item.name.localeCompare(b.item.name);
    });

    // Return top 30
    return results.slice(0, 30).map(function(r) { return r.item; });
  }

  function renderResults(results, container) {
    currentResults = results;
    selectedIndex = results.length > 0 ? 0 : -1;

    if (!results.length) {
      container.innerHTML = '<div class="p-4 text-gray-500 text-sm text-center">No results found</div>';
      return;
    }

    var html = results.map(function(item, index) {
      var typeClass = item.type === 'function'
        ? 'bg-blue-500/20 text-blue-300 border-blue-500/30'
        : item.type === 'operator'
          ? 'bg-pink-500/20 text-pink-300 border-pink-500/30'
          : 'bg-teal-500/20 text-teal-300 border-teal-500/30';
      var typeLabel = item.type === 'function' ? 'fn' : item.type === 'operator' ? 'op' : 'kw';
      var selectedClass = index === selectedIndex ? 'bg-white/10' : '';

      return '<a href="' + item.url + '" data-index="' + index + '" class="search-result flex items-center gap-3 px-4 py-3 hover:bg-white/10 transition-colors cursor-pointer border-b border-white/5 last:border-0 ' + selectedClass + '">' +
        '<span class="text-xs font-mono px-1.5 py-0.5 rounded border ' + typeClass + '">' + typeLabel + '</span>' +
        '<div class="flex-1 min-w-0">' +
          '<div class="font-mono text-sm text-white">' + item.name + '</div>' +
          '<div class="text-xs text-gray-500 truncate">' + item.description + '</div>' +
        '</div>' +
        '<span class="text-xs px-2 py-0.5 rounded bg-white/5 text-gray-400">' + item.category + '</span>' +
      '</a>';
    }).join('');

    container.innerHTML = html;
  }

  function updateSelection() {
    var container = document.getElementById('docs-search-results');
    if (!container) return;

    var items = container.querySelectorAll('.search-result');
    items.forEach(function(item, index) {
      if (index === selectedIndex) {
        item.classList.add('bg-white/10');
        item.scrollIntoView({ block: 'nearest' });
      } else {
        item.classList.remove('bg-white/10');
      }
    });
  }

  function navigateResults(direction) {
    if (!currentResults.length) return;

    if (direction === 'down') {
      selectedIndex = Math.min(selectedIndex + 1, currentResults.length - 1);
    } else if (direction === 'up') {
      selectedIndex = Math.max(selectedIndex - 1, 0);
    }

    updateSelection();
  }

  function goToSelected() {
    if (selectedIndex >= 0 && selectedIndex < currentResults.length) {
      window.location.href = currentResults[selectedIndex].url;
      closeSearch();
    }
  }

  function openSearch() {
    var modal = document.getElementById('docs-search-modal');
    var input = document.getElementById('docs-search-input');
    if (modal && input) {
      modal.classList.remove('hidden');
      input.focus();
    }
  }

  function closeSearch() {
    var modal = document.getElementById('docs-search-modal');
    var input = document.getElementById('docs-search-input');
    var results = document.getElementById('docs-search-results');
    if (modal) modal.classList.add('hidden');
    if (input) input.value = '';
    if (results) results.innerHTML = '';
  }

  function handleSearch() {
    var input = document.getElementById('docs-search-input');
    var resultsContainer = document.getElementById('docs-search-results');
    if (!input || !resultsContainer) return;

    var query = input.value;
    if (!query.trim()) {
      resultsContainer.innerHTML = '';
      return;
    }

    var results = search(query);
    renderResults(results, resultsContainer);
  }

  // Initialize when DOM is ready
  document.addEventListener('DOMContentLoaded', function() {
    var input = document.getElementById('docs-search-input');
    if (input) {
      input.addEventListener('input', handleSearch);
    }

    // Click outside to close
    var modal = document.getElementById('docs-search-modal');
    if (modal) {
      modal.addEventListener('click', function(e) {
        if (e.target === modal) {
          closeSearch();
        }
      });
    }

    // Click result to close
    var resultsContainer = document.getElementById('docs-search-results');
    if (resultsContainer) {
      resultsContainer.addEventListener('click', function(e) {
        if (e.target.closest('a')) {
          closeSearch();
        }
      });
    }
  });

  // Keyboard shortcuts
  document.addEventListener('keydown', function(e) {
    var modal = document.getElementById('docs-search-modal');
    var isOpen = modal && !modal.classList.contains('hidden');

    // Close with ESC
    if (e.key === 'Escape' && isOpen) {
      e.preventDefault();
      closeSearch();
      return;
    }

    // Arrow navigation when modal is open
    if (isOpen && e.key === 'ArrowDown') {
      e.preventDefault();
      navigateResults('down');
      return;
    }

    if (isOpen && e.key === 'ArrowUp') {
      e.preventDefault();
      navigateResults('up');
      return;
    }

    // Enter to go to selected result
    if (isOpen && e.key === 'Enter') {
      e.preventDefault();
      goToSelected();
      return;
    }

    // Open with / key (unless in input/textarea)
    if (e.key === '/' && !isOpen && !['INPUT', 'TEXTAREA'].includes(document.activeElement.tagName) && !document.activeElement.isContentEditable) {
      e.preventDefault();
      openSearch();
      return;
    }

    // Open/close with Cmd+K / Ctrl+K
    if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
      e.preventDefault();
      if (isOpen) {
        closeSearch();
      } else {
        openSearch();
      }
      return;
    }
  });
})();
