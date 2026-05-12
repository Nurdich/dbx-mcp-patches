'use client';

type Testimonial = {
  name: string;
  role: string;
  quote: string;
  avatar: string;
};

type InfiniteMovingCardsProps = {
  items: Testimonial[];
  direction?: 'left' | 'right';
  speed?: 'slow' | 'normal';
};

export function InfiniteMovingCards({ items, direction = 'left', speed = 'normal' }: InfiniteMovingCardsProps) {
  const repeatedItems = [...items, ...items];

  return (
    <div
      className="landing-testimonial-marquee"
      data-direction={direction}
      data-speed={speed}
      aria-label="Testimonials"
    >
      <div className="landing-testimonial-track">
        {repeatedItems.map((item, index) => (
          <figure className="landing-testimonial-card" key={`${item.name}-${index}`}>
            <div className="landing-testimonial-author">
              <img className="landing-testimonial-avatar" src={item.avatar} alt="" width={42} height={42} />
              <figcaption>
                <strong>{item.name}</strong>
                <span>{item.role}</span>
              </figcaption>
            </div>
            <blockquote>{item.quote}</blockquote>
          </figure>
        ))}
      </div>
    </div>
  );
}
