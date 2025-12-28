document.addEventListener('DOMContentLoaded', () => {
    const slides = document.querySelectorAll('.slide');
    const progressBar = document.querySelector('.progress-bar');
    const prevBtn = document.getElementById('prev-btn');
    const nextBtn = document.getElementById('next-btn');
    
    let currentSlide = 0;
    const totalSlides = slides.length;

    // Initialize
    updateSlide(currentSlide);

    function updateSlide(index) {
        // Bounds check
        if (index < 0) index = 0;
        if (index >= totalSlides) index = totalSlides - 1;
        
        currentSlide = index;

        // Update slides visibility
        slides.forEach((slide, i) => {
            if (i === currentSlide) {
                slide.classList.add('active');
            } else {
                slide.classList.remove('active');
            }
        });

        // Update progress bar
        const progress = ((currentSlide + 1) / totalSlides) * 100;
        progressBar.style.width = `${progress}%`;

        // Update URL hash without scrolling
        history.replaceState(null, null, `#slide-${currentSlide + 1}`);
    }

    function nextSlide() {
        if (currentSlide < totalSlides - 1) {
            updateSlide(currentSlide + 1);
        }
    }

    function prevSlide() {
        if (currentSlide > 0) {
            updateSlide(currentSlide - 1);
        }
    }

    // Event Listeners
    if (prevBtn) prevBtn.addEventListener('click', prevSlide);
    if (nextBtn) nextBtn.addEventListener('click', nextSlide);

    document.addEventListener('keydown', (e) => {
        switch(e.key) {
            case 'ArrowRight':
            case 'ArrowDown':
            case 'Space':
            case ' ':
            case 'Enter':
                nextSlide();
                break;
            case 'ArrowLeft':
            case 'ArrowUp':
                prevSlide();
                break;
        }
    });

    // Check for hash on load
    const hash = window.location.hash;
    if (hash && hash.startsWith('#slide-')) {
        const slideIndex = parseInt(hash.replace('#slide-', '')) - 1;
        if (!isNaN(slideIndex)) {
            updateSlide(slideIndex);
        }
    }
});
