# Licence paragraphs for the GitHub release notes

The first paragraph goes into the notes of 0.7.5 and every later release. The second goes into the notes of each earlier release that has a Linux AppImage, whose bundles carry no offer.

## 0.7.5 and later

### Licences

Every installer and the Linux tarball carry the `licenses/` folder: `THIRD-PARTY-NOTICES.md`, with the notes and licence files for ported code, sample data, special cases and the system libraries of the Linux AppImage, and the complete lists of the Rust crates, including the Rust standard library (`THIRD-PARTY-LICENSES-RUST.md`), and of the npm packages in the app (`THIRD-PARTY-LICENSES-NPM.md`), each with its licence texts. Every installer and the Linux tarball also carry the Tycho-2 star catalogue of the offline plate solver (`catalogs/tycho2-l6-v1.abcat`); `tycho2-catalogue-NOTICE.txt` in the `licenses/` folder gives its source, its acknowledgement and the two statements of its terms that its sources give, CC BY-NC 3.0 IGO from the CDS and ESA and BSD-2-Clause from Debian. The Linux AppImage also carries system libraries from Ubuntu 22.04 LTS: WebKitGTK, GTK 3, libsoup 3 and other libraries under the GNU LGPL; its `licenses/appimage-libraries/` folder lists each of them with its package version and holds the copyright file of each package. Written offer: for as long as this AppImage is offered for download and for at least three years after the last day it is offered, Samuel Krieger Bonini, the AstroBurst maintainer, will give anyone who asks, free of charge, the complete corresponding source code of those LGPL libraries in the exact versions this AppImage contains. To ask, open an issue at https://github.com/samuelkriegerbonini-dev/AstroBurst/issues and name the release. The .deb and .rpm packages and the Linux tarball use your system's own copies of these libraries.

## Earlier releases with a Linux AppImage

### Licences of the Linux AppImage

The Linux AppImage on this page carries system libraries from Ubuntu 22.04 LTS that its own notices do not mention: WebKitGTK, GTK 3, libsoup 3 and other libraries under the GNU Lesser General Public License, most of them version 2.1 or later (text: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html). Written offer: for as long as that AppImage is offered for download and for at least three years after the last day it is offered, Samuel Krieger Bonini, the AstroBurst maintainer, will give anyone who asks, free of charge, the complete corresponding source code of those LGPL libraries in the exact versions that AppImage contains. To ask, open an issue at https://github.com/samuelkriegerbonini-dev/AstroBurst/issues and name the release. `src-tauri/licenses/THIRD-PARTY-NOTICES.md` in the AstroBurst repository, from 0.7.5 on, explains how the libraries and their versions are identified. The other Linux packages on this page use your system's own copies of these libraries.
