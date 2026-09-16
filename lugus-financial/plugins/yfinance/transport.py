"""Disable dependency request deadlines only for explicitly unlimited research."""

_research_session = None


def ticker_for_research(yf, symbol, unlimited_research):
    global _research_session
    if not unlimited_research and _research_session is None:
        return yf.Ticker(symbol)
    if _research_session is None:
        from curl_cffi.requests import Session

        class ResearchSession(Session):
            unlimited_research = False

            def request(self, method, url, **kwargs):
                # yfinance supplies separate timezone/cookie/crumb deadlines.
                if self.unlimited_research:
                    kwargs['timeout'] = None
                return super().request(method, url, **kwargs)

        _research_session = ResearchSession(impersonate='chrome')
    # yfinance shares its session globally. Reset its mode on every operation,
    # including a bounded operation after an unlimited initialization.
    _research_session.unlimited_research = unlimited_research
    return yf.Ticker(symbol, session=_research_session)
