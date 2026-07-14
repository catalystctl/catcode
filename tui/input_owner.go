package main

// suspendComposer temporarily moves the user's chat draft out of the shared
// composer while a blocking interaction needs that surface. Attachments remain
// owned by the draft and are restored with it.
func (s *session) suspendComposer(owner string) {
	if owner == "" {
		return
	}
	for _, d := range s.composerDrafts {
		if d.owner == owner {
			return
		}
	}
	d := composerDraft{
		owner: owner, text: s.input.Value(), cursor: s.input.Position(),
		images: append([]string(nil), s.pendingImages...),
	}
	s.composerDrafts = append(s.composerDrafts, d)
	s.input.Reset()
	s.pendingImages = nil
	s.closeMention()
}

func (s *session) restoreComposer(owner string) {
	idx := -1
	for i := len(s.composerDrafts) - 1; i >= 0; i-- {
		if s.composerDrafts[i].owner == owner {
			idx = i
			break
		}
	}
	if idx < 0 {
		return
	}
	d := s.composerDrafts[idx]
	s.composerDrafts = append(s.composerDrafts[:idx], s.composerDrafts[idx+1:]...)
	// Nested blockers restore only when the draft being released is the top
	// owner. Otherwise the remaining blocker still owns the composer.
	if idx != len(s.composerDrafts) {
		return
	}
	s.input.SetValue(d.text)
	s.input.SetCursor(d.cursor)
	s.pendingImages = d.images
	s.input.Focus()
	_ = s.evalMention()
}

func (s *session) restoreAllComposerDrafts() {
	if len(s.composerDrafts) == 0 {
		return
	}
	d := s.composerDrafts[0]
	s.composerDrafts = nil
	s.input.SetValue(d.text)
	s.input.SetCursor(d.cursor)
	s.pendingImages = d.images
	s.input.Focus()
	_ = s.evalMention()
}

func (s *session) enqueueIntercom(p *intercomPrompt) {
	if p == nil || p.requestID == "" {
		return
	}
	if s.pendingIntercom != nil && s.pendingIntercom.requestID == p.requestID {
		return
	}
	for _, queued := range s.intercomQueue {
		if queued.requestID == p.requestID {
			return
		}
	}
	if s.pendingIntercom == nil {
		s.suspendComposer("intercom")
		s.pendingIntercom = p
		s.input.Focus()
		return
	}
	s.intercomQueue = append(s.intercomQueue, p)
}

func (s *session) advanceIntercom() {
	s.input.Reset()
	if len(s.intercomQueue) > 0 {
		s.pendingIntercom = s.intercomQueue[0]
		s.intercomQueue = s.intercomQueue[1:]
		s.input.Focus()
		return
	}
	s.pendingIntercom = nil
	s.restoreComposer("intercom")
}
