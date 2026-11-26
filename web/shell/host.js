// The site Mitten runs in. A site embedding Mitten replaces this file with
// its own; anything left out keeps the default below (see web/README.md).
window.mittenHost = {
	// The product's name in the loading screen's messages.
	name: 'Mitten',
	// The topbar's leading logo and name. `icon` is one of Mitten's own
	// assets (assets/), drawn on the 640×540 brand canvas.
	brand: { name: 'Mitten', icon: 'brand/mitten.svg' },
	// Library switcher and library settings. A site serving one library of
	// its own turns them off.
	libraries: true,
	// Topbar height in pixels (38 on the desktop).
	topbarHeight: 38,
	// Where error messages link for help: an article per kind of failure.
	help: {
		browser: 'https://github.com/pinaltech/Mitten/blob/main/web/README.md',
		start: 'https://github.com/pinaltech/Mitten/issues'
	},
	// Where the page goes when the photo API answers 401, and to sign out.
	signIn: null,
	signOut: null
};
