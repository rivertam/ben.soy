/** Browser events, keyboard navigation, and DOM rendering over Rust search. */
import { core } from '/airport-search.js'

function enhanceCombobox(root) {
  const input = root.querySelector('input')
  if (!input) return

  let options = []
  let open = false
  let highlighted = 0
  let list = null

  function close() {
    open = false
    root.classList.remove('is-open')
    list?.remove()
    list = null
    input.removeAttribute('aria-expanded')
    input.removeAttribute('aria-activedescendant')
  }

  function select(airport) {
    input.value = airport.iata
    close()
    input.blur()
  }

  function renderList() {
    if (!open || options.length === 0) {
      close()
      return
    }
    if (!list) {
      list = document.createElement('ul')
      list.className = 'combobox-list'
      list.role = 'listbox'
      root.appendChild(list)
    }
    list.replaceChildren(
      ...options.map((airport, i) => {
        const li = document.createElement('li')
        li.role = 'option'
        li.id = `${input.id}-opt-${i}`
        li.dataset.iata = airport.iata
        li.setAttribute('aria-selected', i === highlighted ? 'true' : 'false')
        li.innerHTML =
          `<span class="opt-main">${escapeHtml(airport.city)}, ${escapeHtml(airport.country)}</span>` +
          `<span class="opt-code">${escapeHtml(airport.iata)}</span>`
        li.addEventListener('mousedown', (e) => {
          e.preventDefault()
          select(airport)
        })
        li.addEventListener('mouseenter', () => {
          highlighted = i
          syncHighlight()
        })
        return li
      }),
    )
    root.classList.add('is-open')
    input.setAttribute('aria-expanded', 'true')
    input.setAttribute('aria-controls', list.id || (list.id = `${input.id}-listbox`))
    syncHighlight()
  }

  function syncHighlight() {
    if (!list) return
    const items = list.querySelectorAll('[role=option]')
    items.forEach((el, i) => {
      el.setAttribute('aria-selected', i === highlighted ? 'true' : 'false')
    })
    const active = items[highlighted]
    if (active) {
      input.setAttribute('aria-activedescendant', active.id)
      active.scrollIntoView({ block: 'nearest' })
    }
  }

  function onInput() {
    const q = input.value
    options = JSON.parse(core.airport_search(q))
    highlighted = 0
    // Always show hits while typing — including when the query is already an
    // exact IATA ("mia" → MIA). Hiding on exact match made city-code queries
    // look broken; how-bad keeps the list open whenever there are results.
    open = options.length > 0
    if (open) renderList()
    else close()
  }

  input.addEventListener('input', onInput)
  input.addEventListener('focus', (e) => e.target.select())
  input.addEventListener('blur', () => close())
  input.addEventListener('keydown', (e) => {
    if (!open) {
      if (e.key === 'Escape') input.blur()
      return
    }
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      highlighted = Math.min(highlighted + 1, options.length - 1)
      syncHighlight()
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      highlighted = Math.max(highlighted - 1, 0)
      syncHighlight()
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const chosen = options[highlighted]
      if (chosen) select(chosen)
    } else if (e.key === 'Escape') {
      e.preventDefault()
      close()
      input.blur()
    }
  })
}

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
}

/**
 * The docked strip's height varies with the route's via fields as they wrap,
 * so the static --strip-h guesses in planes-tokens.css only fit routes whose
 * fields pack into one row. Feed the real box back as --strip-h-measured
 * (which the token declarations prefer); without JS the guesses still apply,
 * calibrated to the packed nonstop dock — a no-JS multi-row route degrades
 * to sticky offsets that sit a little high.
 */
function trackStripHeight() {
  const strip = document.querySelector('.dispatch .form-dock')
  const dispatch = document.querySelector('.dispatch')
  if (!strip || !dispatch) return
  const apply = () => {
    dispatch.style.setProperty('--strip-h-measured', `${strip.getBoundingClientRect().height}px`)
  }
  if (typeof ResizeObserver !== 'undefined') new ResizeObserver(apply).observe(strip)
  apply()
}

async function boot() {
  trackStripHeight()
  document
    .querySelectorAll('[data-airport-combobox]')
    .forEach((root) => enhanceCombobox(root))
}

boot()
