const home = document.querySelector("[data-felix-home]");
const hero = document.querySelector(".felix-hero");
const age = document.querySelector("[data-felix-age]");
const dogAge = document.querySelector("[data-felix-dog-age]");

if (age) {
  const [year, month, day] = age.dataset.birthday.split("-").map(Number);
  const birthday = new Date(year, month - 1, day);
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  let years = today.getFullYear() - birthday.getFullYear();
  let months = today.getMonth() - birthday.getMonth();

  if (today.getDate() < birthday.getDate()) months -= 1;
  if (months < 0) {
    years -= 1;
    months += 12;
  }

  const parts = [`${years} ${years === 1 ? "year" : "years"}`];
  if (months) parts.push(`${months} ${months === 1 ? "month" : "months"}`);
  age.textContent = `${parts.join(", ")} old`;

  if (dogAge) {
    const yearsInDogYears = ((today - birthday) / (365.25 * 24 * 60 * 60 * 1000)) * 7;
    dogAge.textContent = `${yearsInDogYears.toFixed(1)} dog years`;
  }
}

if (home && hero) {
  const observer = new IntersectionObserver(
    ([entry]) => home.classList.toggle("is-visible", entry.intersectionRatio < 0.98),
    { threshold: [0.98] },
  );

  observer.observe(hero);
}

// The lightbox is plain dialog UI. It used to mirror the open photo into the
// address bar (pushState per photo, a synthesized /felix entry under deep
// links so Back closed into the gallery); that machinery is gone — the same
// gallery now also renders inside the home pane deck, where rewriting the
// URL to /felix would be a lie. Deep links (/felix/{slug}) still open on the
// right photo via data-felix-gallery-initial.
const gallery = document.querySelector("[data-felix-gallery]");

if (gallery) {
  const dialog = document.querySelector("[data-felix-gallery-dialog]");
  const triggers = [...gallery.querySelectorAll("[data-felix-gallery-trigger]")];
  const close = dialog?.querySelector("[data-felix-gallery-close]");
  const previous = dialog?.querySelector("[data-felix-gallery-prev]");
  const next = dialog?.querySelector("[data-felix-gallery-next]");
  const image = dialog?.querySelector("[data-felix-gallery-image]");
  const position = dialog?.querySelector("[data-felix-gallery-position]");
  const caption = dialog?.querySelector("[data-felix-gallery-caption]");
  const initialSlug = gallery.dataset.felixGalleryInitial;
  let currentIndex = 0;
  let returnFocus = null;

  const indexForSlug = (slug) => triggers.findIndex((trigger) => trigger.dataset.felixGallerySlug === slug);

  const setPhoto = (index) => {
    currentIndex = Math.max(0, Math.min(index, triggers.length - 1));
    const trigger = triggers[currentIndex];
    const { felixGalleryAlt: alt, felixGalleryCaption: text, felixGallerySrc: src, felixGalleryStamp: stamp } = trigger.dataset;

    image.src = src;
    image.alt = alt;
    position.textContent = `${stamp} · photo ${currentIndex + 1} of ${triggers.length}`;
    caption.textContent = text;
    caption.hidden = !text;
    previous.disabled = currentIndex === 0;
    next.disabled = currentIndex === triggers.length - 1;
  };

  const openPhoto = (index, { returnTo } = {}) => {
    if (returnTo !== undefined) returnFocus = returnTo;
    setPhoto(index);
    if (!dialog.open) {
      dialog.showModal();
      close.focus();
    }
    document.documentElement.classList.add("felix-lightbox-open");
    document.body.classList.add("felix-lightbox-open");
  };

  triggers.forEach((trigger, index) => {
    trigger.addEventListener("click", () => {
      openPhoto(index, { returnTo: trigger });
    });
  });

  previous.addEventListener("click", () => openPhoto(currentIndex - 1));
  next.addEventListener("click", () => openPhoto(currentIndex + 1));
  close.addEventListener("click", () => dialog.close());

  dialog.addEventListener("click", (event) => {
    if (event.target === dialog) dialog.close();
  });

  dialog.addEventListener("keydown", (event) => {
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      openPhoto(currentIndex - 1);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      openPhoto(currentIndex + 1);
    } else if (event.key === "Home") {
      event.preventDefault();
      openPhoto(0);
    } else if (event.key === "End") {
      event.preventDefault();
      openPhoto(triggers.length - 1);
    }
  });

  dialog.addEventListener("close", () => {
    image.removeAttribute("src");
    image.alt = "";
    document.documentElement.classList.remove("felix-lightbox-open");
    document.body.classList.remove("felix-lightbox-open");
    returnFocus?.focus();
    returnFocus = null;
  });

  const initialIndex = indexForSlug(initialSlug);
  if (initialIndex >= 0) openPhoto(initialIndex);
}
