(function() {
    console.log('[TextraOverlay] overlay.js: Script execution started.');

    // Ensure textraApp namespace exists
    window.textraApp = window.textraApp || {};
    console.log('[TextraOverlay] overlay.js: window.textraApp namespace ensured.');

    // --- STATE ---
    window.textraApp.state = {
        isOverlayVisible: false,
        currentSearchTerm: '',
        templates: [], // Will be [{ name: "Category", rules: [{ triggers:[], replacement:"", description:"" }] }]
        filteredTemplates: [],
        selectedCategoryIndex: 0,
        selectedTemplateIndex: 0,
        style: { // Default styles, will be overwritten by initConfig
            width: 800,
            height: 600,
            font_size: 14,
            font_family: 'Segoe UI, Tahoma, Geneva, Verdana, sans-serif',
            opacity: 0.95,
            primary_color: '#3498db', // Blue
            secondary_color: '#2c3e50', // Dark Blue/Grey
            text_color: '#ecf0f1', // Light Grey/White
            border_radius: 8,
        },
        error: null,
        isLoading: true,
    };
    console.log('[TextraOverlay] overlay.js: Initial state defined.');

    // --- PERFORMANCE OPTIMIZATIONS ---
    window.textraApp.searchDebounce = null;
    console.log('[TextraOverlay] overlay.js: searchDebounce initialized.');

    // --- METHODS ---

    window.textraApp.applyStyles = function() {
        console.log('[TextraOverlay] overlay.js: applyStyles called.');
        const overlayPanel = document.querySelector('.overlay-panel');
        if (overlayPanel) {
            overlayPanel.style.setProperty('--primary-color', this.state.style.primary_color);
            overlayPanel.style.setProperty('--secondary-color', this.state.style.secondary_color);
            overlayPanel.style.setProperty('--text-color', this.state.style.text_color);
            overlayPanel.style.setProperty('--font-family', this.state.style.font_family);
            overlayPanel.style.setProperty('--font-size', `${this.state.style.font_size}px`);
            overlayPanel.style.setProperty('--border-radius', `${this.state.style.border_radius}px`);
            // Opacity is handled by the main window in Rust for Windows
        }
        document.body.style.fontFamily = this.state.style.font_family;
        document.body.style.fontSize = `${this.state.style.font_size}px`;
        console.log('[TextraOverlay] overlay.js: Styles applied.');
    };

    window.textraApp.render = function() {
        console.log('[TextraOverlay] overlay.js: render called.');
        const container = document.getElementById('templates-container');
        if (!container) {
            console.error('[TextraOverlay] overlay.js: templates-container not found!');
            return;
        }

        container.innerHTML = ''; // Clear previous content

        if (this.state.isLoading) {
            container.innerHTML = '<div class="loading-message">Loading templates...</div>';
            console.log('[TextraOverlay] overlay.js: Rendered loading state.');
            return;
        }

        if (this.state.error) {
            container.innerHTML = `<div class="error-message">Error: ${this.state.error}</div>`;
            console.log('[TextraOverlay] overlay.js: Rendered error state:', this.state.error);
            return;
        }

        if (this.state.filteredTemplates.length === 0) {
            container.innerHTML = '<div class="empty-message">No templates found.</div>';
            console.log('[TextraOverlay] overlay.js: Rendered empty state.');
            return;
        }

        const fragment = document.createDocumentFragment();
        this.state.filteredTemplates.forEach((category, catIndex) => {
            const categoryDiv = document.createElement('div');
            categoryDiv.className = 'template-category';
            
            const categoryTitle = document.createElement('h2');
            categoryTitle.textContent = category.name;
            categoryDiv.appendChild(categoryTitle);

            const ul = document.createElement('ul');
            category.rules.forEach((rule, ruleIndex) => {
                const li = document.createElement('li');
                li.dataset.categoryIndex = catIndex;
                li.dataset.ruleIndex = ruleIndex;
                li.tabIndex = 0; // Make it focusable

                const triggerSpan = document.createElement('span');
                triggerSpan.className = 'template-trigger';
                triggerSpan.textContent = rule.triggers.join(', ');
                li.appendChild(triggerSpan);

                const descriptionSpan = document.createElement('span');
                descriptionSpan.className = 'template-description';
                descriptionSpan.textContent = rule.description || rule.replacement.substring(0, 100) + (rule.replacement.length > 100 ? '...' : '');
                li.appendChild(descriptionSpan);
                
                li.addEventListener('click', () => {
                    this.state.selectedCategoryIndex = catIndex;
                    this.state.selectedTemplateIndex = ruleIndex;
                    this.selectItem();
                });
                li.addEventListener('keydown', (event) => { // Allow selection with Enter/Space on focused item
                    if (event.key === 'Enter' || event.key === ' ') {
                        this.state.selectedCategoryIndex = catIndex;
                        this.state.selectedTemplateIndex = ruleIndex;
                        this.selectItem();
                        event.preventDefault();
                    }
                });

                ul.appendChild(li);
            });
            categoryDiv.appendChild(ul);
            fragment.appendChild(categoryDiv);
        });

        container.appendChild(fragment);
        this.updateSelection(); // Highlight the current selection
        console.log('[TextraOverlay] overlay.js: Rendered templates.');
    };
    
    window.textraApp.filterAndCategorizeTemplates = function(searchTerm) {
        const term = searchTerm.toLowerCase();
        if (!term) {
            return this.state.templates; // Return all if no search term
        }
        const filtered = [];
        this.state.templates.forEach(category => {
            const matchingRules = category.rules.filter(rule => 
                rule.triggers.some(trigger => trigger.toLowerCase().includes(term)) ||
                (rule.description && rule.description.toLowerCase().includes(term)) ||
                rule.replacement.toLowerCase().includes(term)
            );
            if (matchingRules.length > 0) {
                filtered.push({ ...category, rules: matchingRules });
            }
        });
        return filtered;
    };

    window.textraApp.initConfig = function(newConfig) {
        console.log('[TextraOverlay] overlay.js: initConfig function CALLED. Received config:', JSON.stringify(newConfig).substring(0, 500) + "...");
        try {
            if (!newConfig || typeof newConfig !== 'object') {
                console.error('[TextraOverlay] overlay.js: initConfig received invalid newConfig (not an object or null).', newConfig);
                this.state.error = 'Invalid configuration data received (type error).';
                this.state.isLoading = false;
                this.render();
                return;
            }
            if (!newConfig.categories || !Array.isArray(newConfig.categories)) {
                console.error('[TextraOverlay] overlay.js: initConfig received invalid newConfig (missing or invalid categories).', newConfig.categories);
                this.state.error = 'Invalid configuration data: categories missing or not an array.';
                this.state.isLoading = false;
                this.render();
                return;
            }
            if (!newConfig.style || typeof newConfig.style !== 'object') {
                console.error('[TextraOverlay] overlay.js: initConfig received invalid newConfig (missing or invalid style).', newConfig.style);
                this.state.error = 'Invalid configuration data: style missing or not an object.';
                // Continue with default styles if possible, but log error
            }

            this.state.templates = newConfig.categories;
            if (newConfig.style) { // Only update style if provided and valid
                 this.state.style = { ...this.state.style, ...newConfig.style };
            }
            this.state.filteredTemplates = this.filterAndCategorizeTemplates(this.state.currentSearchTerm);
            this.state.isLoading = false;
            this.state.error = null;
            
            console.log('[TextraOverlay] overlay.js: initConfig processed. Applying styles and rendering.');
            this.applyStyles();
            this.render();
        } catch (e) {
            console.error('[TextraOverlay] overlay.js: Critical error in initConfig:', e.message, e.stack);
            this.state.error = 'Error processing configuration: ' + e.message;
            this.state.isLoading = false;
            this.render(); // Try to render the error
        }
    };
    console.log('[TextraOverlay] overlay.js: initConfig method defined.');

    window.textraApp.showOverlay = function() {
        console.log('[TextraOverlay] overlay.js: showOverlay called.');
        const overlay = document.getElementById('overlay');
        if (overlay) {
            overlay.classList.add('visible');
            overlay.classList.remove('hidden');
            this.state.isOverlayVisible = true;
            const searchInput = document.getElementById('search-input');
            if (searchInput) {
                searchInput.value = ''; // Clear search on show
                this.state.currentSearchTerm = '';
                this.state.filteredTemplates = this.filterAndCategorizeTemplates('');
                this.render();
                searchInput.focus();
            }
            this.state.selectedCategoryIndex = 0;
            this.state.selectedTemplateIndex = 0;
            this.updateSelection();
            console.log('[TextraOverlay] overlay.js: Overlay shown.');
        } else {
            console.error('[TextraOverlay] overlay.js: Overlay element not found in showOverlay.');
        }
    };
    console.log('[TextraOverlay] overlay.js: showOverlay method defined.');

    window.textraApp.hideOverlay = function() {
        console.log('[TextraOverlay] overlay.js: hideOverlay called.');
        const overlay = document.getElementById('overlay');
        if (overlay) {
            overlay.classList.remove('visible');
            overlay.classList.add('hidden');
            this.state.isOverlayVisible = false;
            // Optionally, tell Rust to hide the window if Rust isn't already doing it
            if (window.external && typeof window.external.invoke === 'function') {
                 console.log('[TextraOverlay] overlay.js: Invoking CloseOverlay to Rust.');
                 window.external.invoke(JSON.stringify({ type: 'CloseOverlay' }));
            }
            console.log('[TextraOverlay] overlay.js: Overlay hidden.');
        } else {
            console.error('[TextraOverlay] overlay.js: Overlay element not found in hideOverlay.');
        }
    };
    console.log('[TextraOverlay] overlay.js: hideOverlay method defined.');

    window.textraApp.handleSearch = function(event) {
        this.state.currentSearchTerm = event.target.value;
        console.log('[TextraOverlay] overlay.js: handleSearch, term:', this.state.currentSearchTerm);
        clearTimeout(this.searchDebounce);
        this.searchDebounce = setTimeout(() => {
            this.state.filteredTemplates = this.filterAndCategorizeTemplates(this.state.currentSearchTerm);
            this.state.selectedCategoryIndex = 0; // Reset selection
            this.state.selectedTemplateIndex = 0;
            this.render();
            console.log('[TextraOverlay] overlay.js: Search debounced, rendered.');
        }, 250); // 250ms debounce
    };
    console.log('[TextraOverlay] overlay.js: handleSearch method defined.');
    
    window.textraApp.updateSelection = function() {
        console.log(`[TextraOverlay] overlay.js: updateSelection - Cat: ${this.state.selectedCategoryIndex}, Tpl: ${this.state.selectedTemplateIndex}`);
        const items = document.querySelectorAll('#templates-container li');
        items.forEach(item => item.classList.remove('selected'));

        if (this.state.filteredTemplates.length > 0 && 
            this.state.filteredTemplates[this.state.selectedCategoryIndex] &&
            this.state.filteredTemplates[this.state.selectedCategoryIndex].rules[this.state.selectedTemplateIndex]) {
            
            // Find the correct DOM element based on data attributes, as direct indexing might be tricky
            // This is a bit inefficient but more robust if DOM structure changes slightly
            let currentOverallIndex = 0;
            let targetOverallIndex = 0;
            for(let i=0; i < this.state.selectedCategoryIndex; i++) {
                targetOverallIndex += this.state.filteredTemplates[i].rules.length;
            }
            targetOverallIndex += this.state.selectedTemplateIndex;

            const targetItem = items[targetOverallIndex];
            if (targetItem) {
                targetItem.classList.add('selected');
                targetItem.focus(); // Ensure the selected item is focusable and focused
                // Scroll into view if necessary
                targetItem.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
                console.log('[TextraOverlay] overlay.js: Item selected and focused:', targetItem);
            } else {
                 console.warn('[TextraOverlay] overlay.js: Target item for selection not found in DOM.');
            }
        } else {
            console.log('[TextraOverlay] overlay.js: No item to select or selection out of bounds.');
        }
    };
    console.log('[TextraOverlay] overlay.js: updateSelection method defined.');

    window.textraApp.selectItem = function() {
        console.log('[TextraOverlay] overlay.js: selectItem called.');
        if (this.state.filteredTemplates.length > 0 &&
            this.state.filteredTemplates[this.state.selectedCategoryIndex] &&
            this.state.filteredTemplates[this.state.selectedCategoryIndex].rules[this.state.selectedTemplateIndex]) {
            
            const selectedRule = this.state.filteredTemplates[this.state.selectedCategoryIndex].rules[this.state.selectedTemplateIndex];
            console.log('[TextraOverlay] overlay.js: Selected rule:', selectedRule);
            if (window.external && typeof window.external.invoke === 'function') {
                window.external.invoke(JSON.stringify({ type: 'TemplateSelected', data: { text: selectedRule.replacement } }));
                console.log('[TextraOverlay] overlay.js: TemplateSelected invoked to Rust.');
            } else {
                console.error('[TextraOverlay] overlay.js: window.external.invoke not available to send TemplateSelected.');
            }
            this.hideOverlay(); // Hide after selection
        } else {
            console.warn('[TextraOverlay] overlay.js: No item to select in selectItem.');
        }
    };
    console.log('[TextraOverlay] overlay.js: selectItem method defined.');

    window.textraApp.handleKeyDown = function(event) {
        if (!this.state.isOverlayVisible) return;
        console.log('[TextraOverlay] overlay.js: handleKeyDown - Key:', event.key);

        let preventDefault = false;
        const numCategories = this.state.filteredTemplates.length;
        if (numCategories === 0 && event.key !== 'Escape') return; // No items to navigate besides Esc

        switch (event.key) {
            case 'Escape':
                this.hideOverlay();
                preventDefault = true;
                break;
            case 'ArrowDown':
                if (numCategories > 0) {
                    const rulesInCurrentCategory = this.state.filteredTemplates[this.state.selectedCategoryIndex].rules.length;
                    if (this.state.selectedTemplateIndex < rulesInCurrentCategory - 1) {
                        this.state.selectedTemplateIndex++;
                    } else if (this.state.selectedCategoryIndex < numCategories - 1) {
                        this.state.selectedCategoryIndex++;
                        this.state.selectedTemplateIndex = 0;
                    }
                    this.updateSelection();
                }
                preventDefault = true;
                break;
            case 'ArrowUp':
                if (numCategories > 0) {
                    if (this.state.selectedTemplateIndex > 0) {
                        this.state.selectedTemplateIndex--;
                    } else if (this.state.selectedCategoryIndex > 0) {
                        this.state.selectedCategoryIndex--;
                        const rulesInPrevCategory = this.state.filteredTemplates[this.state.selectedCategoryIndex].rules.length;
                        this.state.selectedTemplateIndex = rulesInPrevCategory - 1;
                    }
                    this.updateSelection();
                }
                preventDefault = true;
                break;
            case 'Enter':
                 if (document.activeElement && document.activeElement.tagName === 'LI') {
                    // If a list item is directly focused (e.g. by tabbing or updateSelection)
                    this.selectItem();
                } else if (numCategories > 0) { // Fallback if focus is on search bar
                    this.selectItem();
                }
                preventDefault = true;
                break;
            case 'Tab':
                // Basic tab navigation: search -> first item -> search ...
                // More complex trapping might be needed for full accessibility.
                const searchInput = document.getElementById('search-input');
                const firstItem = document.querySelector('#templates-container li');
                if (document.activeElement === searchInput && firstItem) {
                    firstItem.focus();
                    preventDefault = true;
                } else if (document.activeElement !== searchInput && searchInput) {
                    searchInput.focus();
                    preventDefault = true;
                }
                break;
        }
        if (preventDefault) {
            event.preventDefault();
        }
    };
    console.log('[TextraOverlay] overlay.js: handleKeyDown method defined.');

    window.textraApp.initKeyboardNavigation = function() {
        console.log('[TextraOverlay] overlay.js: initKeyboardNavigation called.');
        document.addEventListener('keydown', (event) => this.handleKeyDown(event));
        console.log('[TextraOverlay] overlay.js: Global keydown listener attached.');
    };
    console.log('[TextraOverlay] overlay.js: initKeyboardNavigation method defined.');

    // --- GLOBAL READY FUNCTION ---
    window.isTextraAppReady = function() {
        // console.log('[TextraOverlay] overlay.js: isTextraAppReady CALLED'); // Can be too noisy
        const ready = !!(window.textraApp && typeof window.textraApp.initConfig === 'function' && typeof window.textraApp.render === 'function');
        // if (!ready) {
        //    console.warn('[TextraOverlay] overlay.js: isTextraAppReady check FAILED. textraApp:', !!window.textraApp, 'initConfig:', typeof window.textraApp.initConfig, 'render:', typeof window.textraApp.render);
        // }
        return ready;
    };
    console.log('[TextraOverlay] overlay.js: isTextraAppReady function defined globally.');

    // --- DOMContentLoaded for DOM-dependent setup ---
    document.addEventListener('DOMContentLoaded', function() {
        console.log('[TextraOverlay] overlay.js: DOMContentLoaded event fired.');
        
        const searchInput = document.getElementById('search-input');
        if (searchInput) {
            searchInput.addEventListener('input', (event) => window.textraApp.handleSearch(event));
            console.log('[TextraOverlay] overlay.js: Search input event listener attached.');
        } else {
            console.error('[TextraOverlay] overlay.js: Search input element not found on DOMContentLoaded.');
        }

        const closeButton = document.getElementById('close-button');
        if (closeButton) {
            closeButton.addEventListener('click', () => window.textraApp.hideOverlay());
            console.log('[TextraOverlay] overlay.js: Close button event listener attached.');
        } else {
            console.error('[TextraOverlay] overlay.js: Close button element not found on DOMContentLoaded.');
        }
        
        if (window.textraApp && typeof window.textraApp.initKeyboardNavigation === 'function') {
            window.textraApp.initKeyboardNavigation(); // Sets up document-level keydown listener
        } else {
            console.error('[TextraOverlay] overlay.js: textraApp.initKeyboardNavigation not found on DOMContentLoaded. Cannot init keyboard nav.');
        }
        
        // Initial render call if not loading (e.g. if initConfig was somehow missed or for empty state)
        if (window.textraApp && !window.textraApp.state.isLoading) {
            console.log('[TextraOverlay] overlay.js: DOMContentLoaded - Performing an initial render as not in loading state.');
            window.textraApp.applyStyles(); // Apply any default or pre-loaded styles
            window.textraApp.render();
        } else {
            console.log('[TextraOverlay] overlay.js: DOMContentLoaded - State is still loading or textraApp not fully ready for render.');
        }
    });
    console.log('[TextraOverlay] overlay.js: DOMContentLoaded listener logic defined.');

    console.log('[TextraOverlay] overlay.js: Script execution finished. window.textraApp should be populated.');
})();