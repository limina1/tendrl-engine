;;; tendrl.el --- Emacs client for a running tendrl engine, with a nak fallback  -*- lexical-binding: t; -*-

;; Author: limina1
;; Version: 0.1.0
;; Package-Requires: ((emacs "28.1"))
;; Keywords: comm, hypermedia, nostr
;; URL: https://github.com/limina1/tendrl-engine

;;; Commentary:

;; The engine-facing half of the tendrl Emacs tooling.  `nostrdown-mode'
;; sits on top of it.  This file owns:
;;
;; - the HTTP client for the engine (`tendrl-request', `tendrl-engine-available-p')
;;   and the `nak fetch' fallback (`tendrl-nak-fetch');
;; - pulling one event by nevent / naddr / note / hex id / coordinate:
;;     M-x tendrl-local-fetch   — from the local nostrdb only (never the network)
;;     M-x tendrl-relay-fetch   — from the relays through the engine
;;                                (C-u: through nak directly, bypassing the engine)
;;     M-x tendrl-fetch         — local first, relays if missing
;;   Each opens the event's *content* in a read-only buffer, rendered in the
;;   major mode its `format' tag names (Org, Markdown, AsciiDoc, else text),
;;   with `nostrdown-mode' on so its own references resolve.
;; - the raw event, kept apart from the rendered content: `j' in that buffer
;;   (or M-x tendrl-event-json anywhere) opens the event JSON, pretty-printed,
;;   in its own read-only buffer — the content view never mixes the two.
;;
;; The buffer keeps the event as `tendrl-event' (an alist) so other commands
;; can inspect it; `R' re-fetches the same event from the relays, `w' copies
;; its id.

;;; Code:

(require 'json)
(require 'url)
(require 'url-http)
(require 'subr-x)
(require 'seq)
(require 'thingatpt)

(defvar url-http-end-of-headers)
(defvar nostrdown-coord)
(defvar nostrdown-author)
(defvar nostrdown-siblings)
(defvar nostrdown-mode)
(declare-function nostrdown-mode "nostrdown-mode")
(declare-function nostrdown-resolve-buffer "nostrdown-mode")

;;;; Customisation

(defgroup tendrl nil
  "Client for a running tendrl engine."
  :group 'comm
  :prefix "tendrl-")

(defcustom tendrl-engine-url "http://localhost:3030"
  "Base URL of a running tendrl engine."
  :type 'string)

(defcustom tendrl-nak-program "nak"
  "The nak executable used when the engine is bypassed or unreachable."
  :type 'string)

(defcustom tendrl-nak-relays nil
  "Relays handed to `nak fetch' besides the entity's own hints.
When the engine is reachable its fetch relay set is added too, so a bare
nevent / naddr with no hints still has somewhere to look."
  :type '(repeat string))

(defcustom tendrl-request-timeout 8
  "Seconds to wait for a synchronous engine request."
  :type 'integer)

(defcustom tendrl-json-mode (cond ((fboundp 'js-json-mode) 'js-json-mode)
                                  ((fboundp 'json-mode) 'json-mode)
                                  (t 'js-mode))
  "Major mode for the raw event JSON buffer."
  :type 'function)

;;;; HTTP client

(defvar tendrl--engine-alive nil
  "Cons of (TIME . ALIVE-P) from the last `/health' probe.")

(defun tendrl--url (path)
  (concat (string-remove-suffix "/" tendrl-engine-url) path))

(defun tendrl--parse-json (body)
  "Parse BODY for the resolver: arrays as lists, null and false as nil."
  (json-parse-string body :object-type 'alist :array-type 'list
                     :null-object nil :false-object nil))

(defun tendrl--parse-event-json (body)
  "Parse BODY for the event view: arrays as lists, null and false kept
distinct (`:null' / `:false') so the raw JSON round-trips faithfully."
  (json-parse-string body :object-type 'alist :array-type 'list
                     :null-object :null :false-object :false))

(defun tendrl--parse-response (&optional parser)
  "Parse the JSON body of the `url-retrieve' buffer with PARSER.
Return (STATUS . DATA)."
  (goto-char (point-min))
  (let ((status (and (re-search-forward "^HTTP/[0-9.]+ \\([0-9]+\\)" nil t)
                     (string-to-number (match-string 1))))
        (body-start (cond ((and (boundp 'url-http-end-of-headers)
                                (markerp url-http-end-of-headers))
                           (1+ (marker-position url-http-end-of-headers)))
                          ((progn (goto-char (point-min))
                                  (re-search-forward "^\r?$" nil t))
                           (1+ (point))))))
    (cons status
          (when (and body-start (<= body-start (point-max)))
            (condition-case nil
                (funcall (or parser #'tendrl--parse-json)
                         (decode-coding-string
                          (buffer-substring-no-properties body-start (point-max)) 'utf-8))
              (error nil))))))

(defun tendrl-request (method path payload callback &optional sync parser)
  "Send METHOD PATH with JSON PAYLOAD; call CALLBACK with (STATUS DATA).
STATUS is nil when the engine did not answer.  With SYNC, block for up to
`tendrl-request-timeout' seconds.  PARSER (default `tendrl--parse-json')
turns the body into DATA."
  (let ((url-request-method method)
        (url-request-extra-headers
         (and payload '(("Content-Type" . "application/json"))))
        (url-request-data
         (and payload (encode-coding-string (json-serialize payload) 'utf-8)))
        (url-show-status nil)
        (url (tendrl--url path)))
    (if sync
        (let ((buf (condition-case nil
                       (url-retrieve-synchronously url t t tendrl-request-timeout)
                     (error nil))))
          (if (not (buffer-live-p buf))
              (funcall callback nil nil)
            (unwind-protect
                (let ((r (with-current-buffer buf (tendrl--parse-response parser))))
                  (funcall callback (car r) (cdr r)))
              (kill-buffer buf))))
      (condition-case nil
          (url-retrieve
           url
           (lambda (status)
             (let ((buf (current-buffer)))
               (unwind-protect
                   (if (plist-get status :error)
                       (funcall callback nil nil)
                     (let ((r (tendrl--parse-response parser)))
                       (funcall callback (car r) (cdr r))))
                 (when (buffer-live-p buf) (kill-buffer buf)))))
           nil t t)
        (error (funcall callback nil nil))))))

(defun tendrl-engine-available-p ()
  "Non-nil when the engine answered `/health' within the last 30 seconds."
  (let ((now (float-time)))
    (when (or (null tendrl--engine-alive)
              (> (- now (car tendrl--engine-alive)) 30))
      (let ((tendrl-request-timeout 2) (alive nil))
        (tendrl-request "GET" "/health" nil
                        (lambda (status _) (setq alive (eq status 200)))
                        t)
        (setq tendrl--engine-alive (cons now alive))))
    (cdr tendrl--engine-alive)))

;;;; nak fallback

(defvar tendrl--engine-relays nil
  "Cons of (TIME . URLS): the engine's fetch relay set, cached a minute.")

(defun tendrl-engine-fetch-relays ()
  "The engine's fetch relay URLs, or nil when it does not answer."
  (let ((now (float-time)))
    (when (or (null tendrl--engine-relays)
              (> (- now (car tendrl--engine-relays)) 60))
      (let ((urls nil))
        (when (tendrl-engine-available-p)
          (tendrl-request "GET" "/api/v1/relays" nil
                          (lambda (status data)
                            (when (eq status 200)
                              (setq urls (alist-get 'urls (alist-get 'fetch data)))))
                          t))
        (setq tendrl--engine-relays (cons now urls))))
    (cdr tendrl--engine-relays)))

(defun tendrl-nak-fetch (entity callback)
  "Run `nak fetch ENTITY' asynchronously.
CALLBACK receives the first event as an alist, or nil."
  (let ((buf (generate-new-buffer " *tendrl-nak*")))
    (condition-case nil
        ;; nak blocks on an open stdin: hand it EOF right away.
        (process-send-eof
         (make-process
          :name "tendrl-nak" :buffer buf :noquery t
          :command (append (list tendrl-nak-program "fetch")
                           (mapcan (lambda (r) (list "--relay" r))
                                   (delete-dups (append tendrl-nak-relays
                                                        (tendrl-engine-fetch-relays))))
                           (list (string-remove-prefix "nostr:" entity)))
          :sentinel
          (lambda (proc _event)
            (unless (process-live-p proc)
              (let ((event
                     (with-current-buffer buf
                       (goto-char (point-min))
                       (when (re-search-forward "^{" nil t)
                         (condition-case nil
                             (json-parse-string
                              (buffer-substring-no-properties
                               (match-beginning 0) (line-end-position))
                              :object-type 'alist :array-type 'list
                              :null-object nil :false-object nil)
                           (error nil))))))
                (kill-buffer buf)
                (funcall callback event))))))
      (error (kill-buffer buf) (funcall callback nil)))))

;;;; Event helpers

(defun tendrl-event-tag (event name)
  "First value of tag NAME on EVENT, or nil."
  (seq-some (lambda (tg) (and (equal (car tg) name) (cadr tg)))
            (alist-get 'tags event)))

(defun tendrl-event-title (event)
  "Title of EVENT: its `title' tag, or a kind-0's display name."
  (or (tendrl-event-tag event "title")
      (when (eql (alist-get 'kind event) 0)
        (condition-case nil
            (let ((m (json-parse-string (alist-get 'content event)
                                        :object-type 'alist :null-object nil)))
              ;; A blank display_name is common; fall through to name.
              (seq-some (lambda (k) (let ((v (alist-get k m)))
                                      (and (stringp v) (not (string-blank-p v))
                                           (string-trim v))))
                        '(display_name name)))
          (error nil)))))

(defun tendrl-event-coord (event)
  "The `kind:pubkey:d-tag' coordinate of EVENT, when it carries a d tag."
  (when-let* ((d (tendrl-event-tag event "d")))
    (format "%s:%s:%s" (alist-get 'kind event) (alist-get 'pubkey event) d)))

(defconst tendrl-kind-formats
  '((30818 . "asciidoc")   ; wiki article (NIP-54)
    (30817 . "markdown")   ; wiki article, markdown flavour
    (30023 . "markdown")   ; long-form (NIP-23)
    (30041 . "asciidoc"))  ; publication section (NKBIP-01) — unless its tags say otherwise
  "Default content format per event kind, used when the event names none.")

(defun tendrl-format-mode (fmt)
  "Major mode for the content format name or mimetype FMT, or nil.
`org' / `text/org', `markdown' / `md' / `text/markdown' and `asciidoc' /
`adoc' / `text/asciidoc' are recognised; an unknown format, or one whose
mode is not installed, yields nil."
  (let ((f (downcase (or fmt ""))))
    (cond ((string-match-p "org" f) (and (fboundp 'org-mode) 'org-mode))
          ((string-match-p "markdown\\|\\bmd\\b" f)
           (and (fboundp 'markdown-mode) 'markdown-mode))
          ((string-match-p "asciidoc\\|adoc" f)
           (and (fboundp 'adoc-mode) 'adoc-mode)))))

(defun tendrl-event-format (event)
  "The content format EVENT declares (its `format' / `m' / `M' tag),
else the default for its kind from `tendrl-kind-formats', else nil."
  (or (seq-some (lambda (tg) (and (member (car tg) '("format" "m" "M")) (cadr tg)))
                (alist-get 'tags event))
      (alist-get (alist-get 'kind event) tendrl-kind-formats)))

(defun tendrl-event-major-mode (event)
  "Major mode for EVENT's content.
Its declared format wins, then its kind's default (`tendrl-event-format');
a kind-0 profile is JSON; anything else is `text-mode', and
`tendrl-event-set-format' applies a markup layer by hand."
  (cond ((tendrl-format-mode (tendrl-event-format event)))
        ((eql (alist-get 'kind event) 0) tendrl-json-mode)
        (t 'text-mode)))

;;;; Identifiers

(defconst tendrl-ident-regexp
  (concat "\\(?:nostr:\\)?"
          "\\(?:\\(?:naddr\\|nevent\\|note\\|npub\\|nprofile\\)1[a-z0-9]+"
          "\\|[0-9a-fA-F]\\{64\\}"
          "\\|[0-9]+:[0-9a-fA-F]\\{64\\}:[^][ \t\n|}]+\\)")
  "An event / profile identifier: NIP-19 entity, 64-hex id, or coordinate.")

(defun tendrl-ident-at-point ()
  "The identifier under point, if any."
  (when (thing-at-point-looking-at tendrl-ident-regexp 200)
    (string-remove-prefix "nostr:" (match-string-no-properties 0))))

(defun tendrl-read-ident (prompt)
  "Read an identifier, defaulting to the one at point."
  (let* ((default (tendrl-ident-at-point))
         (s (read-string (if default (format "%s(%s…) " prompt (substring default 0 (min 14 (length default))))
                           prompt)
                         nil 'tendrl-ident-history default)))
    (string-remove-prefix "nostr:" (string-trim s))))

(defun tendrl--ident-kind (ident)
  "Classify IDENT.
One of `hex', `coord', `naddr', `nevent', `note', `npub', `nprofile', or nil."
  (cond ((string-match-p "\\`[0-9a-fA-F]\\{64\\}\\'" ident) 'hex)
        ((string-match-p "\\`[0-9]+:[0-9a-fA-F]\\{64\\}:" ident) 'coord)
        ((string-match "\\`\\(naddr\\|nevent\\|note\\|npub\\|nprofile\\)1" ident)
         (intern (match-string 1 ident)))))

;;;; Fetching

(defun tendrl-fetch-event (ident policy callback)
  "Fetch the event IDENT names through the engine with fetch POLICY.
IDENT is a hex id, nevent, note, naddr or a `kind:pubkey:d-tag'
coordinate.  CALLBACK receives the event alist, or nil with the failure
already reported."
  (let* ((fail (lambda (why) (message "tendrl: %s" why) (funcall callback nil)))
        (events (lambda (id)
                  (tendrl-request
                   "GET" (format "/api/v1/events/%s?policy=%s" id policy) nil
                   (lambda (status data)
                     (let ((ev (and (eq status 200) (or (alist-get 'event data)
                                                        (and (alist-get 'id data) data)))))
                       (if ev (funcall callback ev)
                         (funcall fail (format "event %s… not found (%s, %s)"
                                               (substring id 0 8) policy (or status "no engine"))))))
                   nil #'tendrl--parse-event-json)))
        (addressable (lambda (kind pk d)
                       (tendrl-request
                        "GET" (format "/api/v1/addressable/%s/%s/%s?policy=%s"
                                      kind pk (url-hexify-string d) policy)
                        nil
                        (lambda (status data)
                          (let ((ev (and (eq status 200) (alist-get 'event data))))
                            (if ev (funcall callback ev)
                              (funcall fail (format "%s:%s…:%s not found (%s, %s)"
                                                    kind (substring pk 0 8) d policy
                                                    (or status "no engine"))))))
                        nil #'tendrl--parse-event-json))))
    (pcase (tendrl--ident-kind ident)
      ('hex (funcall events (downcase ident)))
      ('coord (let* ((parts (split-string ident ":"))
                     (kind (car parts)) (pk (cadr parts))
                     (d (string-join (cddr parts) ":")))
                (funcall addressable kind pk d)))
      ((or 'naddr 'nevent 'note)
       (tendrl-request
        "POST" "/api/v1/decode" `((input . ,ident))
        (lambda (status data)
          (cond
           ((not (eq status 200)) (funcall fail (format "could not decode %s" ident)))
           ((equal (alist-get 'kind data) "naddr")
            (funcall addressable (alist-get 'kind_int data) (alist-get 'pubkey data)
                     (alist-get 'd_tag data)))
           ((alist-get 'event_id data) (funcall events (alist-get 'event_id data)))
           (t (funcall fail (format "%s is not an event pointer" ident)))))))
      ((or 'npub 'nprofile) (funcall fail "that is a profile, not an event — use tendrl-profile"))
      (_ (funcall fail (format "not an identifier: %s" ident))))))

(defun tendrl-fetch-profile (pubkey-or-entity policy callback)
  "Fetch the kind-0 profile for a hex PUBKEY-OR-ENTITY (npub / nprofile).
CALLBACK receives the profile alist, or nil."
  (let ((go (lambda (pk)
              (tendrl-request
               "GET" (format "/api/v1/profile/%s?policy=%s" pk policy) nil
               (lambda (status data)
                 (if (eq status 200) (funcall callback data)
                   (message "tendrl: profile %s… not loaded (%s)" (substring pk 0 8) status)
                   (funcall callback nil)))))))
    (if (eq (tendrl--ident-kind pubkey-or-entity) 'hex)
        (funcall go pubkey-or-entity)
      (tendrl-request
       "POST" "/api/v1/decode" `((input . ,pubkey-or-entity))
       (lambda (status data)
         (if (and (eq status 200) (alist-get 'pubkey data))
             (funcall go (alist-get 'pubkey data))
           (message "tendrl: could not decode %s" pubkey-or-entity)
           (funcall callback nil)))))))

;;;; The event view

(defvar-local tendrl-event nil
  "The raw event (alist) shown in this buffer.")
(defvar-local tendrl-event-source nil
  "Where `tendrl-event' came from: \"local\", \"relays\", or \"nak\".")
(defvar-local tendrl-event-ident nil
  "The identifier the event was fetched by.")

(defvar tendrl-event-view-mode-map
  (let ((m (make-sparse-keymap)))
    (define-key m (kbd "j") #'tendrl-event-json)
    (define-key m (kbd "R") #'tendrl-event-refetch-from-relays)
    (define-key m (kbd "w") #'tendrl-event-copy-id)
    (define-key m (kbd "m") #'tendrl-event-set-format)
    (define-key m (kbd "q") #'quit-window)
    m)
  "Keys in an event content buffer.")

(define-minor-mode tendrl-event-view-mode
  "Read-only view of one Nostr event's content.
`j' shows the raw JSON in a separate buffer, `R' re-fetches from the relays,
`w' copies the event id, `m' re-renders in another markup, `q' quits."
  :lighter " Event"
  :keymap tendrl-event-view-mode-map)

(defun tendrl--header-line (event source)
  (format " %s · kind %s · %s… · %s · %s    j: json  R: relays  w: copy id  m: format  q: quit"
          (or (tendrl-event-title event) "untitled")
          (alist-get 'kind event)
          (substring (or (alist-get 'pubkey event) "????????") 0 8)
          (format-time-string "%Y-%m-%d %H:%M" (or (alist-get 'created_at event) 0))
          (or source "local")))

(defun tendrl--render-event (buf event source ident mode)
  "Fill BUF with EVENT's content under major MODE plus the event view.
SOURCE and IDENT are kept as `tendrl-event-source' / `tendrl-event-ident'."
  (with-current-buffer buf
    (let ((inhibit-read-only t))
      (erase-buffer)
      (insert (or (alist-get 'content event) ""))
      (when (and (eq mode tendrl-json-mode) (fboundp 'json-pretty-print-buffer))
        (ignore-errors (json-pretty-print-buffer)))
      (goto-char (point-min)))
    ;; A major mode kills the buffer-locals, so everything is set after it.
    (funcall mode)
    (setq tendrl-event event
          tendrl-event-source source
          tendrl-event-ident ident
          header-line-format (tendrl--header-line event source))
    (when (require 'nostrdown-mode nil t)
      (setq nostrdown-coord (tendrl-event-coord event)
            nostrdown-author (alist-get 'pubkey event)
            nostrdown-siblings nil))
    (view-mode 1)
    (tendrl-event-view-mode 1)
    (when (and (fboundp 'nostrdown-mode) (not (eq mode tendrl-json-mode)))
      (nostrdown-mode 1)
      (nostrdown-resolve-buffer)))
  buf)

(defun tendrl-show-event (event &optional source ident)
  "Display EVENT's content read-only, in the major mode its format names
\(see `tendrl-event-major-mode').
SOURCE labels where it came from; IDENT is what it was fetched by."
  (let* ((title (or (tendrl-event-title event) (format "kind %s" (alist-get 'kind event))))
         (buf (get-buffer-create (format "*tendrl: %s*" title))))
    (tendrl--render-event buf event source ident (tendrl-event-major-mode event))
    (pop-to-buffer buf)
    buf))

(defun tendrl-event-set-format (fmt)
  "Re-render this event buffer as FMT: asciidoc, markdown, org or text.
For an event whose kind and tags name no format (shown in `text-mode'),
this applies the markup layer by hand; nostrdown overlays are re-resolved."
  (interactive
   (list (completing-read "Render as: " '("asciidoc" "markdown" "org" "text") nil t
                          nil nil (tendrl-event-format tendrl-event))))
  (unless tendrl-event (user-error "Not in a tendrl event buffer"))
  (let ((mode (or (tendrl-format-mode fmt) 'text-mode)))
    (when (and (eq mode 'text-mode) (not (equal fmt "text")))
      (message "tendrl: no major mode installed for %s; using text-mode" fmt))
    (tendrl--render-event (current-buffer) tendrl-event tendrl-event-source
                          tendrl-event-ident mode)))

(defun tendrl-show-profile (profile &optional source)
  "Display a kind-0 PROFILE alist in a read-only buffer."
  (let ((buf (get-buffer-create
              (format "*tendrl: @%s*" (or (alist-get 'name profile) (alist-get 'pubkey profile))))))
    (with-current-buffer buf
      (let ((inhibit-read-only t))
        (erase-buffer)
        (dolist (k '(display_name name nip05 about picture website lud16 pubkey))
          (when-let* ((v (alist-get k profile)))
            (insert (format "%-13s %s\n" (symbol-name k) v))))
        (goto-char (point-min)))
      (setq tendrl-event profile
            tendrl-event-source source
            header-line-format (format " profile · %s… · %s    j: json  q: quit"
                                       (substring (or (alist-get 'pubkey profile) "????????") 0 8)
                                       (or source "local")))
      (view-mode 1)
      (tendrl-event-view-mode 1))
    (pop-to-buffer buf)
    buf))

;;;; Raw JSON, kept apart from the rendered content

(defun tendrl--json-ready (v)
  "Make V (a parsed event) acceptable to `json-serialize'.
Lists become arrays; an empty list prints as `[]' (a JSON null parsed
without `tendrl--parse-event-json' also lands here); `:null' / `:false'
pass through."
  (cond ((null v) [])
        ((memq v '(:null :false t)) v)
        ((and (consp v) (consp (car v)) (symbolp (caar v)))
         (mapcar (lambda (kv) (cons (car kv) (tendrl--json-ready (cdr kv)))) v))
        ((listp v) (vconcat (mapcar #'tendrl--json-ready v)))
        (t v)))

(defun tendrl-event-json (&optional event)
  "Show the raw JSON of EVENT (default: this buffer's `tendrl-event') in its
own read-only buffer, pretty-printed.  Outside an event buffer, prompt for an
identifier and fetch it locally."
  (interactive)
  (let ((event (or event tendrl-event)))
    (if (null event)
        (tendrl-fetch-event (tendrl-read-ident "Event JSON: ") "local_only"
                            (lambda (ev) (when ev (tendrl-event-json ev))))
      (let* ((id (or (alist-get 'id event) (alist-get 'pubkey event) "event"))
             (buf (get-buffer-create (format "*tendrl json: %s*" (substring id 0 (min 12 (length id)))))))
        (with-current-buffer buf
          (let ((inhibit-read-only t))
            (erase-buffer)
            (insert (json-serialize (tendrl--json-ready event)))
            (json-pretty-print-buffer)
            (goto-char (point-min)))
          (funcall tendrl-json-mode)
          (setq tendrl-event event
                header-line-format (format " raw event %s… · %d bytes    q: quit"
                                           (substring id 0 (min 8 (length id)))
                                           (buffer-size)))
          (view-mode 1)
          (local-set-key (kbd "q") #'quit-window))
        (pop-to-buffer buf)
        buf))))

(defun tendrl-event-copy-id ()
  "Copy this buffer's event id (or pubkey) to the kill ring."
  (interactive)
  (let ((id (or (alist-get 'id tendrl-event) (alist-get 'pubkey tendrl-event))))
    (unless id (user-error "No event in this buffer"))
    (kill-new id)
    (message "tendrl: copied %s" id)))

(defun tendrl-event-refetch-from-relays ()
  "Fetch this buffer's event again, from the relays."
  (interactive)
  (let ((ident (or tendrl-event-ident (alist-get 'id tendrl-event))))
    (unless ident (user-error "No event in this buffer"))
    (tendrl-relay-fetch ident)))

;;;; Commands

;;;###autoload
(defun tendrl-local-fetch (ident)
  "Pull the event IDENT names from the local nostrdb into a read-only buffer.
IDENT is an nevent, naddr, note, 64-hex id, or `kind:pubkey:d-tag'
coordinate; an npub / nprofile opens the profile.  Never touches the network."
  (interactive (list (tendrl-read-ident "Local fetch: ")))
  (unless (tendrl-engine-available-p)
    (user-error "tendrl: no engine at %s (local fetch reads its nostrdb)" tendrl-engine-url))
  (tendrl--fetch-and-show ident "local_only" "local"))

;;;###autoload
(defun tendrl-relay-fetch (ident &optional via-nak)
  "Pull the event IDENT names from the relays into a read-only buffer.
Goes through the engine (which ingests the event; in Confirm mode the
request waits for your approval in the tendrl UI).  With a prefix argument,
or when no engine answers, fetch with `nak' directly instead — VIA-NAK."
  (interactive (list (tendrl-read-ident "Relay fetch: ") current-prefix-arg))
  (if (or via-nak (not (tendrl-engine-available-p)))
      (progn
        (message "tendrl: nak fetch %s…" (substring ident 0 (min 16 (length ident))))
        (tendrl-nak-fetch
         ident
         (lambda (event)
           (cond ((null event) (message "tendrl: nak found nothing for %s" ident))
                 ((eql (alist-get 'kind event) 0)
                  (tendrl-show-event event "nak" ident))
                 (t (tendrl-show-event event "nak" ident))))))
    (message "tendrl: fetching %s… from the relays (Confirm mode waits for approval in tendrl)"
             (substring ident 0 (min 16 (length ident))))
    (tendrl--fetch-and-show ident "fetch_always" "relays")))

;;;###autoload
(defun tendrl-fetch (ident)
  "Pull the event IDENT names, local first and the relays if it is missing."
  (interactive (list (tendrl-read-ident "Fetch: ")))
  (tendrl--fetch-and-show ident "local_first" "local-first"))

(defun tendrl--fetch-and-show (ident policy source)
  (if (memq (tendrl--ident-kind ident) '(npub nprofile))
      (tendrl-fetch-profile ident policy
                            (lambda (p) (when p (tendrl-show-profile p source))))
    (tendrl-fetch-event ident policy
                        (lambda (ev) (when ev (tendrl-show-event ev source ident))))))

;;;###autoload
(defun tendrl-profile (ident)
  "Show the profile for IDENT (npub, nprofile, or hex pubkey), local first."
  (interactive (list (tendrl-read-ident "Profile: ")))
  (tendrl-fetch-profile ident "local_first"
                        (lambda (p) (when p (tendrl-show-profile p "local-first")))))

(provide 'tendrl)
;;; tendrl.el ends here
