// Spolecny skript prototypu: sada ikon, prototypova lista, motiv, kontrast,
// velikost pisma a omezeni pohybu. Neni to kod systemu - jen nastroj pro nahled.
(function () {
  'use strict';
  var root = document.documentElement;

  // ---- Ikony: 24x24, tah 1.75 (styl "linkove, zaoblene"). Jen cesty, barvu da CSS.
  var ICONS = {
    terminal: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M7 9l3 3-3 3M12.5 15H17"/>',
    folder: '<path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 16.5z"/>',
    file: '<path d="M7 3h7l5 5v11a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M14 3v5h5"/>',
    code: '<path d="M7 3h7l5 5v11a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z"/><path d="M10 12l-2 2 2 2M14 12l2 2-2 2"/>',
    image: '<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="9.5" r="1.5"/><path d="M21 16l-5-5-8 9"/>',
    settings: '<circle cx="12" cy="12" r="3"/><path d="M19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3.9a7 7 0 0 0-2-1.2L14 3h-4l-.6 2.6a7 7 0 0 0-2 1.2l-2.3-.9-2 3.4 2 1.5a7 7 0 0 0 0 2.4l-2 1.5 2 3.4 2.3-.9a7 7 0 0 0 2 1.2L10 21h4l.6-2.6a7 7 0 0 0 2-1.2l2.3.9 2-3.4-2-1.5c.1-.4.1-.8.1-1.2z"/>',
    shield: '<path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z"/>',
    shieldcheck: '<path d="M12 3l7.5 3v5.5c0 4.6-3.2 8.3-7.5 9.5-4.3-1.2-7.5-4.9-7.5-9.5V6z"/><path d="M8.5 12l2.5 2.5 4.5-5"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c2.8 3.2 2.8 14.8 0 18M12 3c-2.8 3.2-2.8 14.8 0 18"/>',
    camera: '<path d="M4 8.5A2.5 2.5 0 0 1 6.5 6H8l1.5-2h5L16 6h1.5A2.5 2.5 0 0 1 20 8.5v8a2.5 2.5 0 0 1-2.5 2.5h-11A2.5 2.5 0 0 1 4 16.5z"/><circle cx="12" cy="12.5" r="3.5"/>',
    mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>',
    lock: '<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/>',
    unlock: '<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8 10.5V7.5a4 4 0 0 1 7.7-1.5"/>',
    key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l8-8M16 7l2 2M14 9l1.5 1.5"/>',
    user: '<circle cx="12" cy="8.5" r="4"/><path d="M4.5 20a7.5 7.5 0 0 1 15 0"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
    bell: '<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
    tasks: '<path d="M10 6.5h10M10 12h10M10 17.5h10"/><path d="M3.5 6.5l1.5 1.5 2.5-3M3.5 12l1.5 1.5 2.5-3"/><circle cx="5.5" cy="17.5" r="1.5"/>',
    agent: '<path d="M12 3l1.8 4.6L18.5 9l-4.7 1.8L12 15.5l-1.8-4.7L5.5 9l4.7-1.4z"/><path d="M18 15l.9 2.1L21 18l-2.1.9L18 21l-.9-2.1L15 18l2.1-.9z"/>',
    close: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
    min: '<path d="M6 12h12"/>',
    max: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
    restore: '<rect x="5" y="9" width="10" height="10" rx="2"/><path d="M9 9V7a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2h-2"/>',
    right: '<path d="M9.5 6l6 6-6 6"/>',
    left: '<path d="M14.5 6l-6 6 6 6"/>',
    down: '<path d="M6 9.5l6 6 6-6"/>',
    up: '<path d="M6 14.5l6-6 6 6"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    alert: '<path d="M12 4l9 16H3z"/><path d="M12 10v4.5M12 17.5v.5"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>',
    history: '<path d="M3.5 12a8.5 8.5 0 1 0 2.5-6"/><path d="M3.5 4.5V8H7"/><path d="M12 7.5V12l3 2"/>',
    branch: '<circle cx="6" cy="5.5" r="2"/><circle cx="6" cy="18.5" r="2"/><circle cx="18" cy="8" r="2"/><path d="M6 7.5v9M18 10c0 4-6 3-11.2 7"/>',
    undo: '<path d="M9 7L4.5 11.5 9 16"/><path d="M4.5 11.5H15a5 5 0 0 1 0 10h-3"/>',
    play: '<path d="M8 5.5v13l10-6.5z"/>',
    pause: '<path d="M8.5 5.5v13M15.5 5.5v13"/>',
    stop: '<rect x="6.5" y="6.5" width="11" height="11" rx="2"/>',
    trash: '<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 13h9l1-13"/>',
    copy: '<rect x="8.5" y="8.5" width="11" height="11" rx="2.5"/><path d="M15.5 8.5V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7.5a2 2 0 0 0 2 2h2.5"/>',
    eye: '<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>',
    keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="2.5"/><path d="M6 10h.5M9.5 10h.5M13 10h.5M16.5 10h1M7 14.5h10"/>',
    usb: '<path d="M12 21V5M12 5l-2.5 3M12 5l2.5 3"/><path d="M12 15l-4-2.5V10M12 13l4-2V8.5"/><rect x="6.5" y="8" width="3" height="2"/><circle cx="16" cy="7.5" r="1.2"/>',
    power: '<path d="M12 3.5V12"/><path d="M7 6.5a7.5 7.5 0 1 0 10 0"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4"/>',
    moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
    contrast: '<circle cx="12" cy="12" r="9"/><path d="M12 3a9 9 0 0 1 0 18z" fill="currentColor"/>',
    textsize: '<path d="M3 18l4.5-12L12 18M4.6 14h5.8M14 18l3.3-8.5L20.5 18M15 15.5h4.6"/>',
    apps: '<rect x="4" y="4" width="6.5" height="6.5" rx="2"/><rect x="13.5" y="4" width="6.5" height="6.5" rx="2"/><rect x="4" y="13.5" width="6.5" height="6.5" rx="2"/><rect x="13.5" y="13.5" width="6.5" height="6.5" rx="2"/>',
    monitor: '<rect x="3" y="4" width="18" height="12.5" rx="2.5"/><path d="M8.5 20.5h7M12 16.5v4"/>',
    wifi: '<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.7 16a5 5 0 0 1 6.6 0"/><circle cx="12" cy="19" r=".8"/>',
    volume: '<path d="M4 9.5h3.5L12 5.5v13l-4.5-4H4z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
    battery: '<rect x="2.5" y="7.5" width="17" height="9" rx="2.5"/><path d="M21.5 10.5v3"/><path d="M5.5 10.5h8v3h-8z" fill="currentColor"/>',
    download: '<path d="M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14"/>',
    home: '<path d="M4 11l8-6.5 8 6.5v8.5a1.5 1.5 0 0 1-1.5 1.5H15v-6H9v6H5.5A1.5 1.5 0 0 1 4 19.5z"/>',
    more: '<circle cx="6" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="18" cy="12" r="1.2"/>',
    refresh: '<path d="M20 11.5a8 8 0 0 0-14.5-4.2M4 12.5a8 8 0 0 0 14.5 4.2"/><path d="M5 3.5v4h4M19 20.5v-4h-4"/>',
    filter: '<path d="M4 5.5h16l-6 7.5v5.5l-4 2V13z"/>',
    sidebar: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M9 4v16"/>',
    window: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M3 9h18"/>',
    cloud: '<path d="M7 18.5a4.5 4.5 0 0 1-.5-9 6 6 0 0 1 11.6 1.5A3.75 3.75 0 0 1 17.5 18.5z"/>',
    disk: '<rect x="3" y="5" width="18" height="14" rx="3"/><path d="M3 13h18"/><circle cx="17" cy="16" r=".8"/>',
    pin: '<path d="M9 4h6l-1 5 3 3v2H7v-2l3-3z"/><path d="M12 14v6.5"/>',
    star: '<path d="M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.9-5.2-2.8-5.2 2.8 1-5.9-4.3-4.1 5.9-.8z"/>',
    send: '<path d="M4 12l16-7.5-6 16-2.5-6.5z"/><path d="M11.5 14l8.5-9.5"/>',
    printer: '<path d="M7 9V4h10v5"/><rect x="3.5" y="9" width="17" height="8" rx="2.5"/><path d="M7 14h10v6H7z"/>',
    accessibility: '<circle cx="12" cy="4.5" r="1.8"/><path d="M5 8.5l7 1.5 7-1.5M12 10v4.5l-3.5 6M12 14.5l3.5 6"/>',
    palette: '<path d="M12 3a9 9 0 1 0 0 18c1.2 0 1.8-.9 1.4-2-.5-1.3.3-2.5 1.7-2.5H17a4 4 0 0 0 4-4C21 6.9 17 3 12 3z"/><circle cx="7.5" cy="11.5" r="1"/><circle cx="10" cy="7.5" r="1"/><circle cx="14.5" cy="7.5" r="1"/>',
    network: '<rect x="9" y="3" width="6" height="5" rx="1.5"/><rect x="3" y="16" width="6" height="5" rx="1.5"/><rect x="15" y="16" width="6" height="5" rx="1.5"/><path d="M12 8v4M6 16v-2h12v2"/>',
    gauge: '<path d="M4.5 17a8.5 8.5 0 1 1 15 0"/><path d="M12 13l3.5-4"/><circle cx="12" cy="13" r="1.2"/>',
    diff: '<path d="M8 4v8M4 8h8M13 18h8"/>'
  };
  // Logo: symbol MagnumOpus z design/assets/logo/symbol-fialovy.svg (cesty), barva z tokenu akcentu.
  var SYMBOL = "<path fill-rule=\"evenodd\" d=\"M53.75,70.75 C58.71,64.94 65.19,58.14 71.75,52.75 C78.31,47.36 85.67,42.36 93.31,38.31 C100.95,34.26 112.92,29.50 121.06,27.06 C129.20,24.62 140.78,22.76 146.06,22.06 C151.34,21.36 156.68,21.18 162.00,21.00 C167.32,20.82 215.68,20.82 221.00,21.00 C226.32,21.18 231.66,21.38 236.94,22.06 C242.22,22.74 259.30,26.17 262.94,27.06 C266.57,27.96 270.18,29.01 273.69,30.31 C277.20,31.62 285.94,35.14 291.69,38.31 C297.44,41.48 307.53,48.20 313.25,52.75 C318.97,57.30 324.39,62.29 329.25,67.75 C334.11,73.21 339.45,80.40 343.69,87.31 C347.92,94.22 354.38,108.15 355.69,111.31 C357.00,114.48 358.03,117.76 358.94,121.06 C359.84,124.37 363.19,138.79 363.94,144.06 C364.68,149.33 364.84,154.68 365.00,160.00 C365.16,165.32 364.62,236.62 364.62,236.62 C364.62,236.62 363.38,236.62 363.38,236.62 C363.38,236.62 362.65,235.34 362.31,234.69 C361.97,234.03 310.36,128.47 306.69,122.31 C303.01,116.15 297.45,108.52 294.25,104.75 C291.05,100.98 287.54,97.42 283.69,94.31 C279.83,91.21 268.06,83.54 263.69,81.31 C259.31,79.09 253.87,77.30 249.94,76.06 C246.00,74.82 241.99,73.81 237.94,73.06 C233.88,72.31 226.68,71.34 221.00,71.00 C215.32,70.66 170.68,70.66 165.00,71.00 C159.32,71.34 153.64,71.93 148.06,73.06 C142.49,74.20 132.26,76.91 126.31,79.31 C120.37,81.71 112.33,86.40 109.31,88.31 C106.30,90.23 103.40,92.36 100.75,94.75 C98.10,97.14 91.36,104.09 87.31,109.31 C83.26,114.53 79.63,120.44 76.31,126.31 C72.99,132.19 32.00,217.01 29.69,221.69 C27.38,226.36 23.05,235.28 22.69,235.69 C22.32,236.09 21.31,236.56 21.31,236.56 C21.31,236.56 20.83,148.66 21.00,146.00 C21.17,143.34 21.58,140.69 22.06,138.06 C22.55,135.44 24.37,126.63 26.06,121.06 C27.75,115.49 29.98,109.48 32.31,104.31 C34.64,99.15 37.31,94.12 40.31,89.31 C43.31,84.51 48.79,76.56 53.75,70.75 Z\r\nM85.75,143.75 C87.87,140.92 90.57,138.54 93.31,136.31 C96.06,134.08 100.08,131.52 102.31,130.31 C104.54,129.10 106.91,128.14 109.31,127.31 C111.71,126.48 115.03,125.39 118.00,125.00 C120.97,124.61 128.74,124.78 131.00,125.00 C133.26,125.22 135.52,125.62 137.69,126.31 C139.85,127.01 145.25,129.21 148.69,131.31 C152.12,133.42 156.15,136.48 159.25,139.75 C162.35,143.02 187.62,180.41 188.75,181.25 C189.88,182.09 192.75,182.62 192.75,182.62 C192.75,182.62 196.24,180.22 197.69,178.69 C199.13,177.16 220.61,145.27 223.75,141.75 C226.89,138.23 231.23,134.41 234.31,132.31 C237.40,130.22 241.19,128.45 244.31,127.31 C247.43,126.18 250.70,125.38 254.00,125.00 C257.30,124.62 263.95,124.68 266.94,125.06 C269.92,125.45 272.87,126.26 275.69,127.31 C278.51,128.37 283.03,130.71 285.69,132.31 C288.34,133.92 291.09,135.92 293.25,137.75 C295.41,139.58 297.57,141.71 299.25,143.75 C300.93,145.79 302.34,148.04 303.69,150.31 C305.04,152.58 357.94,254.17 358.69,256.31 C359.44,258.46 359.91,260.73 360.00,263.00 C360.09,265.27 359.59,269.01 358.94,271.94 C358.28,274.87 356.40,280.56 354.69,284.69 C352.98,288.82 350.97,292.84 348.69,296.69 C346.40,300.53 342.78,306.03 339.25,310.25 C335.72,314.47 328.40,321.95 323.25,326.25 C318.10,330.55 312.12,334.56 306.69,337.69 C301.26,340.81 295.55,343.48 289.69,345.69 C283.83,347.89 272.86,350.90 267.94,351.94 C263.02,352.98 255.31,353.85 253.00,354.00 C250.69,354.15 247.36,354.15 246.06,353.94 C244.76,353.72 243.53,353.19 242.31,352.69 C241.10,352.18 238.13,350.98 236.31,349.69 C234.49,348.39 232.23,346.38 230.75,344.25 C229.27,342.12 227.63,337.86 227.06,335.94 C226.50,334.02 226.12,332.00 226.12,330.00 C226.12,328.00 226.51,325.99 227.06,324.06 C227.61,322.14 228.88,318.47 230.31,316.31 C231.75,314.15 234.05,312.11 235.75,310.75 C237.45,309.39 239.28,308.10 241.31,307.31 C243.35,306.53 259.12,304.71 264.94,302.94 C270.76,301.16 278.32,297.53 281.69,295.69 C285.06,293.84 288.70,291.35 291.25,289.25 C293.80,287.15 297.09,283.86 298.25,282.25 C299.41,280.64 300.51,278.88 300.94,276.94 C301.37,275.00 301.32,271.78 300.69,269.31 C300.06,266.85 264.98,198.00 263.69,196.31 C262.39,194.63 260.14,192.52 258.94,192.06 C257.73,191.60 256.27,191.77 255.06,192.06 C253.86,192.35 252.69,192.94 251.75,193.75 C250.81,194.56 223.80,233.46 221.25,236.25 C218.70,239.04 215.90,241.70 212.69,243.69 C209.47,245.67 204.17,248.09 199.94,248.94 C195.71,249.78 189.97,249.38 187.00,249.00 C184.03,248.62 181.09,247.82 178.31,246.69 C175.54,245.55 171.18,243.12 168.75,241.25 C166.32,239.38 164.28,237.04 162.31,234.69 C160.35,232.34 134.19,194.56 133.25,193.75 C132.31,192.94 131.14,192.35 129.94,192.06 C128.73,191.77 127.27,191.77 126.06,192.06 C124.86,192.35 123.68,192.93 122.75,193.75 C121.82,194.57 121.04,196.07 120.31,197.31 C119.58,198.55 84.94,266.85 84.31,269.31 C83.69,271.78 83.28,274.52 84.06,276.94 C84.84,279.36 89.64,285.27 92.75,288.25 C95.86,291.23 99.56,293.58 103.31,295.69 C107.07,297.79 114.23,301.22 120.06,302.94 C125.90,304.66 135.33,304.96 138.94,306.06 C142.55,307.16 147.51,309.53 149.25,310.75 C150.99,311.97 152.46,313.58 153.69,315.31 C154.92,317.04 156.18,320.10 156.69,321.31 C157.19,322.53 157.73,323.76 157.94,325.06 C158.15,326.36 158.66,332.78 157.94,335.94 C157.21,339.10 154.63,343.33 153.69,344.69 C152.74,346.04 151.55,347.23 150.25,348.25 C148.95,349.27 144.50,351.97 142.69,352.69 C140.88,353.41 138.94,353.85 137.00,354.00 C135.06,354.15 127.67,353.62 123.06,352.94 C118.45,352.25 110.92,350.79 105.06,348.94 C99.20,347.09 87.27,342.46 80.31,338.69 C73.36,334.91 65.82,329.47 60.75,325.25 C55.68,321.03 49.09,313.95 46.75,311.25 C44.41,308.55 42.25,305.69 40.31,302.69 C38.37,299.69 31.59,288.66 29.31,282.69 C27.03,276.72 24.99,267.32 25.00,264.00 C25.01,260.68 26.24,257.45 27.31,254.31 C28.39,251.17 30.53,246.92 32.31,243.31 C34.10,239.70 83.63,146.58 85.75,143.75 Z\"/>";
  window.MO_SYMBOL = SYMBOL;
  var LOGO = '<svg viewBox="-8 -12 400 400" class="logo" aria-hidden="true"><g fill="var(--c-accent)">' + SYMBOL + '</g></svg>';
  window.MO_LOGO = LOGO;

  function injectSprite() {
    var s = '<svg xmlns="http://www.w3.org/2000/svg" style="position:absolute;width:0;height:0" aria-hidden="true">';
    Object.keys(ICONS).forEach(function (k) { s += '<symbol id="ic-' + k + '" viewBox="0 0 24 24">' + ICONS[k] + '</symbol>'; });
    s += '</svg>';
    document.body.insertAdjacentHTML('afterbegin', s);
  }
  window.MO_ICONS = Object.keys(ICONS);
  // Pomocnik pro sablony: ic('folder', 's')
  window.ic = function (name, cls) {
    return '<svg class="ic ' + (cls || '') + '" aria-hidden="true"><use href="#ic-' + name + '"/></svg>';
  };

  // ---- Nastaveni nahledu (uklada se, kdyz uloziste jde).
  function load(k, d) { try { return localStorage.getItem('mo.' + k) || d; } catch (e) { return d; } }
  function save(k, v) { try { localStorage.setItem('mo.' + k, v); } catch (e) {} }
  var state = {
    theme: load('theme', 'auto'),       // auto | light | dark
    contrast: load('contrast', 'normal'),
    scale: load('scale', '1'),
    motion: load('motion', 'system')    // system | reduce | full
  };
  // "Podle denni doby": svetly 7:00-19:59, jinak tmavy. System to bude delat
  // podle vychodu a zapadu slunce v nastavenem pasmu; tady jen priblizne.
  function daytime() { var h = new Date().getHours(); return h >= 7 && h < 20 ? 'light' : 'dark'; }
  function apply() {
    root.setAttribute('data-theme', state.theme === 'auto' ? daytime() : state.theme);
    if (state.contrast === 'more') root.setAttribute('data-contrast', 'more'); else root.removeAttribute('data-contrast');
    root.style.setProperty('--text-scale', state.scale);
    if (state.motion === 'system') root.removeAttribute('data-motion'); else root.setAttribute('data-motion', state.motion);
    document.dispatchEvent(new CustomEvent('mo-theme'));
  }
  window.MO_SET = function (k, v) { state[k] = v; save(k, v); apply(); syncControls(); };
  window.MO_STATE = state;
  apply();

  var PAGES = [
    ['index.html', 'Přehled'], ['design-system.html', 'Design systém'], ['okna.html', 'Okna'],
    ['plocha.html', 'Plocha'], ['terminal.html', 'Terminál'], ['soubory.html', 'Soubory'],
    ['nastaveni.html', 'Nastavení'], ['prihlaseni.html', 'Přihlášení'], ['start.html', 'Start']
  ];
  function syncControls() {
    var m = { theme: 'mo-theme-sel', contrast: 'mo-contrast-sel', scale: 'mo-scale-sel', motion: 'mo-motion-sel' };
    Object.keys(m).forEach(function (k) { var el = document.getElementById(m[k]); if (el) el.value = state[k]; });
  }
  function protoBar() {
    var here = location.pathname.split('/').pop() || 'index.html';
    var nav = PAGES.map(function (p) {
      return '<a href="' + p[0] + '"' + (p[0] === here ? ' aria-current="page"' : '') + '>' + p[1] + '</a>';
    }).join('');
    var html = '<div class="proto" role="region" aria-label="Ovládání náhledu (není součást systému)">' +
      '<span class="brand">' + LOGO + 'MagnumOpus-OS <span class="muted" style="font-weight:400">prototyp</span></span>' +
      '<nav aria-label="Prototypy">' + nav + '</nav>' +
      '<div class="ctl">' +
      '<label>' + window.ic('palette', 's') + '<select id="mo-theme-sel" aria-label="Motiv"><option value="auto">Podle denní doby</option><option value="light">Světlý</option><option value="dark">Tmavý</option></select></label>' +
      '<label>' + window.ic('contrast', 's') + '<select id="mo-contrast-sel" aria-label="Kontrast"><option value="normal">Běžný kontrast</option><option value="more">Vysoký kontrast</option></select></label>' +
      '<label>' + window.ic('textsize', 's') + '<select id="mo-scale-sel" aria-label="Velikost písma"><option value="1">100 %</option><option value="1.25">125 %</option><option value="1.5">150 %</option><option value="2">200 %</option></select></label>' +
      '<label>' + window.ic('gauge', 's') + '<select id="mo-motion-sel" aria-label="Pohyb"><option value="system">Pohyb podle systému</option><option value="reduce">Omezit pohyb</option><option value="full">Plný pohyb</option></select></label>' +
      '</div></div>';
    var host = document.getElementById('proto');
    if (host) host.outerHTML = html; else document.body.insertAdjacentHTML('afterbegin', html);
    [['mo-theme-sel', 'theme'], ['mo-contrast-sel', 'contrast'], ['mo-scale-sel', 'scale'], ['mo-motion-sel', 'motion']].forEach(function (p) {
      document.getElementById(p[0]).addEventListener('change', function (e) { window.MO_SET(p[1], e.target.value); });
    });
    syncControls();
  }

  // ---- Pomocnik: radiogroup .seg ovladatelny sipkami (role="radio").
  window.MO_SEG = function (el, onChange) {
    var btns = Array.prototype.slice.call(el.querySelectorAll('[role="radio"]'));
    function pick(b, focus) {
      btns.forEach(function (x) { var on = x === b; x.setAttribute('aria-checked', on); x.tabIndex = on ? 0 : -1; });
      if (focus) b.focus();
      if (onChange) onChange(b.dataset.value, b);
    }
    btns.forEach(function (b, i) {
      b.tabIndex = b.getAttribute('aria-checked') === 'true' ? 0 : -1;
      b.addEventListener('click', function () { pick(b); });
      b.addEventListener('keydown', function (e) {
        var d = (e.key === 'ArrowRight' || e.key === 'ArrowDown') ? 1 : (e.key === 'ArrowLeft' || e.key === 'ArrowUp') ? -1 : 0;
        if (d) { e.preventDefault(); pick(btns[(i + d + btns.length) % btns.length], true); }
      });
    });
  };
  // ---- Pomocnik: prepinac role="switch".
  window.MO_SWITCH = function (el, onChange) {
    el.addEventListener('click', function () {
      var v = el.getAttribute('aria-checked') !== 'true';
      el.setAttribute('aria-checked', v);
      if (onChange) onChange(v, el);
    });
  };
  // ---- Pomocnik: drzeni fokusu v dialogu (Tab cykli uvnitr, Esc = posledni volba).
  window.MO_TRAP = function (dlg, onEsc) {
    function items() {
      return Array.prototype.slice.call(dlg.querySelectorAll('button:not([disabled]), input, select, [tabindex="0"], a[href]'));
    }
    dlg.addEventListener('keydown', function (e) {
      if (e.key === 'Escape' && onEsc) { e.preventDefault(); onEsc(); return; }
      if (e.key !== 'Tab') return;
      var it = items(); if (!it.length) return;
      var first = it[0], last = it[it.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    });
  };

  function boot() { injectSprite(); protoBar(); document.dispatchEvent(new CustomEvent('mo-ready')); }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot); else boot();
})();
