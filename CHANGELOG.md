# Changelog
All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](http://keepachangelog.com/en/1.0.0/)
and this project adheres to [Semantic Versioning](http://semver.org/spec/v2.0.0.html).

### Categories each change fall into

* **Added**: for new features.
* **Changed**: for changes in existing functionality.
* **Deprecated**: for soon-to-be removed features.
* **Removed**: for now removed features.
* **Fixed**: for any bug fixes.
* **Security**: in case of vulnerabilities.


## [Unreleased]


## [0.1.3] - 2025-05-12
### Added
- Add `Listener::wait_timeout`. Allows waiting for a trigger for a max amount of time.
- Add license files to the package/repository.


## [0.1.2] - 2021-07-19
### Fixed
- Don't store multiple `Waker`s for single `Listener`s that are being polled
  multiple times. Instead only store the last `Waker`. Fixes issue #1.


## [0.1.1] - 2020-05-01
### Added
- Add `is_triggered` method to both `Trigger` and `Listener`.


## [0.1.0] - 2020-04-20
First version
