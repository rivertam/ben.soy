/**
 * Client-side airport typeahead for the planes form.
 *
 * Ports ~/how-bad's `airports.ts` search + `AirportCombobox.tsx` behaviour so
 * suggestions never cross a topcoat shard boundary. Progressive enhancement:
 * without this script the inputs are plain text fields that still GET-submit
 * IATA codes.
 *
 * Expects a form (or ancestor) with `data-airports-url` pointing at the
 * bundled `airports.json`, and each field wrapped in
 * `.combobox[data-airport-combobox]`.
 */

const METROS = [
  { code: 'NYC', name: 'New York', aliases: ['New York City'], airports: ['JFK', 'EWR', 'LGA', 'SWF'] },
  { code: 'WAS', name: 'Washington', aliases: ['Washington DC', 'DC'], airports: ['IAD', 'DCA', 'BWI'] },
  { code: 'CHI', name: 'Chicago', airports: ['ORD', 'MDW'] },
  { code: 'HOU', name: 'Houston', airports: ['IAH', 'HOU'] },
  { name: 'Los Angeles', aliases: ['LA'], airports: ['LAX', 'BUR', 'LGB', 'SNA', 'ONT'] },
  { name: 'San Francisco', aliases: ['SF', 'Bay Area'], airports: ['SFO', 'OAK', 'SJC'] },
  { code: 'YTO', name: 'Toronto', airports: ['YYZ', 'YTZ'] },
  { code: 'YMQ', name: 'Montreal', airports: ['YUL'] },
  { code: 'SAO', name: 'São Paulo', airports: ['GRU', 'CGH', 'VCP'] },
  { code: 'RIO', name: 'Rio de Janeiro', airports: ['GIG', 'SDU'] },
  { code: 'BHZ', name: 'Belo Horizonte', airports: ['CNF', 'PLU'] },
  { code: 'BUE', name: 'Buenos Aires', airports: ['EZE', 'AEP'] },
  { code: 'LON', name: 'London', airports: ['LHR', 'LGW', 'LCY', 'STN', 'LTN', 'SEN'] },
  { code: 'PAR', name: 'Paris', airports: ['CDG', 'ORY', 'BVA'] },
  { code: 'MIL', name: 'Milan', aliases: ['Milano'], airports: ['MXP', 'LIN', 'BGY'] },
  { code: 'ROM', name: 'Rome', aliases: ['Roma'], airports: ['FCO', 'CIA'] },
  { code: 'VCE', name: 'Venice', aliases: ['Venezia'], airports: ['VCE', 'TSF'] },
  { name: 'Florence', airports: ['FLR'] },
  { name: 'Munich', aliases: ['München', 'Muenchen'], airports: ['MUC'] },
  { name: 'Cologne', aliases: ['Koeln', 'Bonn'], airports: ['CGN'] },
  { name: 'Vienna', aliases: ['Wien'], airports: ['VIE'] },
  { name: 'Prague', aliases: ['Praha'], airports: ['PRG'] },
  { code: 'STO', name: 'Stockholm', airports: ['ARN', 'NYO', 'BMA', 'VST'] },
  { code: 'REK', name: 'Reykjavik', airports: ['KEF', 'RKV'] },
  { code: 'WAW', name: 'Warsaw', aliases: ['Warszawa'], airports: ['WAW', 'WMI'] },
  { code: 'BUH', name: 'Bucharest', aliases: ['Bucuresti'], airports: ['OTP', 'BBU'] },
  { code: 'MOW', name: 'Moscow', aliases: ['Moskva'], airports: ['SVO', 'DME', 'VKO', 'ZIA'] },
  { code: 'IEV', name: 'Kyiv', aliases: ['Kiev'], airports: ['KBP', 'IEV'] },
  { code: 'IST', name: 'Istanbul', airports: ['IST', 'SAW'] },
  { code: 'TCI', name: 'Tenerife', airports: ['TFN', 'TFS'] },
  { code: 'DXB', name: 'Dubai', airports: ['DXB', 'DWC'] },
  { code: 'THR', name: 'Tehran', airports: ['IKA', 'THR'] },
  { code: 'TYO', name: 'Tokyo', airports: ['HND', 'NRT'] },
  { code: 'OSA', name: 'Osaka', airports: ['KIX', 'ITM', 'UKB'] },
  { code: 'NGO', name: 'Nagoya', airports: ['NGO', 'NKM'] },
  { code: 'SPK', name: 'Sapporo', airports: ['CTS', 'OKD'] },
  { code: 'SEL', name: 'Seoul', airports: ['ICN', 'GMP'] },
  { code: 'BJS', name: 'Beijing', aliases: ['Peking'], airports: ['PEK', 'PKX'] },
  { code: 'SHA', name: 'Shanghai', airports: ['PVG', 'SHA'] },
  { code: 'TPE', name: 'Taipei', airports: ['TPE', 'TSA'] },
  { code: 'JKT', name: 'Jakarta', airports: ['CGK', 'HLP'] },
  { code: 'BKK', name: 'Bangkok', airports: ['BKK', 'DMK'] },
  { name: 'Mumbai', aliases: ['Bombay'], airports: ['BOM'] },
  { name: 'Chennai', aliases: ['Madras'], airports: ['MAA'] },
  { name: 'Kolkata', aliases: ['Calcutta'], airports: ['CCU'] },
  { name: 'Ho Chi Minh City', aliases: ['Saigon'], airports: ['SGN'] },
  { name: 'Guangzhou', aliases: ['Canton'], airports: ['CAN'] },
  { name: 'Yangon', aliases: ['Rangoon'], airports: ['RGN'] },
  { code: 'MEL', name: 'Melbourne', airports: ['MEL', 'AVV'] },
]

function fold(s) {
  return s
    .toLowerCase()
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .replace(/[^\p{L}\p{N}\s/-]/gu, '')
}

function withinOneEdit(token, word) {
  if (Math.abs(token.length - word.length) > 1) return false
  let i = 0
  while (i < token.length && i < word.length && token[i] === word[i]) i++
  if (i === token.length && i === word.length) return true
  if (token.length === word.length) {
    if (token[i] === word[i + 1] && token[i + 1] === word[i]) {
      return token.slice(i + 2) === word.slice(i + 2)
    }
    return token.slice(i + 1) === word.slice(i + 1)
  }
  const [shorter, longer] = token.length < word.length ? [token, word] : [word, token]
  return shorter.slice(i) === longer.slice(i + 1)
}

function tokensMatchWords(tokens, words) {
  const used = new Set()
  let exact = true
  for (const token of tokens) {
    let found = -1
    for (let w = 0; w < words.length; w++) {
      if (!used.has(w) && words[w].startsWith(token)) {
        found = w
        break
      }
    }
    if (found < 0 && token.length >= 4) {
      for (let w = 0; w < words.length; w++) {
        if (!used.has(w) && withinOneEdit(token, words[w].slice(0, token.length))) {
          found = w
          exact = false
          break
        }
      }
    }
    if (found < 0) return null
    used.add(found)
  }
  return { exact }
}

function buildIndex(airports) {
  const index = airports.map((airport) => {
    const city = fold(airport.city)
    const country = fold(airport.country)
    return {
      airport,
      city,
      cityWords: city.split(/[\s/-]+/),
      country,
      countryWords: country.split(/[\s/-]+/),
      nameWords: fold(airport.name).split(/[\s/-]+/),
      iata: airport.iata.toLowerCase(),
      aliases: [],
      aliasWordLists: [],
    }
  })
  const byIata = new Map(index.map((e) => [e.iata, e]))
  for (const metro of METROS) {
    const aliases = [metro.name, ...(metro.aliases ?? [])]
    if (metro.code) aliases.push(metro.code)
    const folded = aliases.map(fold)
    const wordLists = folded.map((a) => a.split(/[\s/-]+/))
    for (const code of metro.airports) {
      const entry = byIata.get(code.toLowerCase())
      if (!entry) continue
      entry.aliases.push(...folded)
      entry.aliasWordLists.push(...wordLists)
    }
  }
  return index
}

function matchQuality(entry, q, tokens) {
  if (entry.iata === q) return 100
  let quality = 0
  if (entry.aliases.includes(q)) quality = 92
  else if (entry.iata.startsWith(q)) quality = 70

  const cityMatch = tokensMatchWords(tokens, entry.cityWords)
  if (cityMatch) {
    const base = entry.city.startsWith(tokens[0]) ? 70 : 62
    quality = Math.max(quality, cityMatch.exact ? base : base - 30)
  }
  for (const words of entry.aliasWordLists) {
    const aliasMatch = tokensMatchWords(tokens, words)
    if (aliasMatch) {
      const base = words[0].startsWith(tokens[0]) ? 70 : 62
      quality = Math.max(quality, aliasMatch.exact ? base : base - 30)
    }
  }
  const nameMatch = tokensMatchWords(tokens, entry.nameWords)
  if (nameMatch) quality = Math.max(quality, nameMatch.exact ? 55 : 28)

  // Country match — same ranking as the Rust port (below name, above substring).
  const countryMatch = tokensMatchWords(tokens, entry.countryWords)
  if (countryMatch) {
    const base = entry.country.startsWith(tokens[0]) ? 45 : 38
    quality = Math.max(quality, countryMatch.exact ? base : base - 15)
  }

  if (
    quality === 0 &&
    q.length >= 3 &&
    (entry.city.includes(q) ||
      entry.nameWords.join(' ').includes(q) ||
      entry.country.includes(q))
  ) {
    quality = 20
  }
  return quality
}

function searchAirports(index, query, limit = 8) {
  const q = fold(query.trim())
  if (q.length === 0) return []
  const tokens = q.split(/[\s/-]+/).filter(Boolean)
  if (tokens.length === 0) return []

  const scored = []
  for (const entry of index) {
    const quality = matchQuality(entry, q, tokens)
    if (quality > 0) scored.push({ airport: entry.airport, quality })
  }
  return scored
    .sort(
      (x, y) =>
        y.quality - x.quality ||
        y.airport.weight - x.airport.weight ||
        x.airport.city.localeCompare(y.airport.city),
    )
    .slice(0, limit)
    .map((s) => s.airport)
}

function enhanceCombobox(root, index) {
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
    options = searchAirports(index, q)
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
  const host = document.querySelector('[data-airports-url]')
  const url = host?.getAttribute('data-airports-url')
  if (!url) return
  const airports = await (await fetch(url)).json()
  const index = buildIndex(airports)
  document
    .querySelectorAll('[data-airport-combobox]')
    .forEach((root) => enhanceCombobox(root, index))
}

boot()
