'use client';

import { motion } from 'motion/react';

export function HeroProductStage() {
  return (
    <motion.div
      className="landing-product"
      initial={{ opacity: 0, y: 28, rotateX: 4, rotateY: -6 }}
      animate={{ opacity: 1, y: 0, rotateX: 0, rotateY: 0 }}
      transition={{ duration: 0.9, delay: 0.12, ease: [0.16, 1, 0.3, 1] }}
    >
      <div className="landing-window-bar">
        <span />
        <span />
        <span />
      </div>
      <img src="/screenshot-dark.png" alt="DBX product screenshot" />
    </motion.div>
  );
}
